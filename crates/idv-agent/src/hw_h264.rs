//! Optional Media Foundation hardware H.264 encoder.
//!
//! The encoder is selected only when Media Foundation exposes a hardware H.264
//! MFT that accepts NV12 system-memory input. Any failure falls back to OpenH264.

use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::core::Interface;

fn h264_nals_from_annexb(data: &[u8]) -> Vec<Vec<u8>> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 {
            if data[i + 2] == 1 {
                starts.push((i, 3));
                i += 3;
                continue;
            }
            if i + 4 <= data.len() && data[i + 2] == 0 && data[i + 3] == 1 {
                starts.push((i, 4));
                i += 4;
                continue;
            }
        }
        i += 1;
    }
    let mut out = Vec::new();
    for (idx, &(start, prefix)) in starts.iter().enumerate() {
        let from = start + prefix;
        let to = starts.get(idx + 1).map(|(s, _)| *s).unwrap_or(data.len());
        if to > from {
            out.push(data[from..to].to_vec());
        }
    }
    if out.is_empty() && !data.is_empty() {
        out.push(data.to_vec());
    }
    out
}

fn rgb_to_nv12(rgb: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut y_plane = vec![0u8; width * height];
    let mut uv_plane = vec![0u8; width * height / 2];
    for row in 0..height {
        for col in 0..width {
            let i = (row * width + col) * 3;
            let r = rgb[i] as f32;
            let g = rgb[i + 1] as f32;
            let b = rgb[i + 2] as f32;
            let y = (0.299 * r + 0.587 * g + 0.114 * b).clamp(0.0, 255.0) as u8;
            y_plane[row * width + col] = y;
        }
    }
    for row in (0..height).step_by(2) {
        for col in (0..width).step_by(2) {
            let mut u_sum = 0.0f32;
            let mut v_sum = 0.0f32;
            for dy in 0..2 {
                for dx in 0..2 {
                    let i = ((row + dy) * width + col + dx) * 3;
                    let r = rgb[i] as f32;
                    let g = rgb[i + 1] as f32;
                    let b = rgb[i + 2] as f32;
                    u_sum += -0.169 * r - 0.331 * g + 0.5 * b + 128.0;
                    v_sum += 0.5 * r - 0.419 * g - 0.081 * b + 128.0;
                }
            }
            let uv = (row / 2) * width + col;
            uv_plane[uv] = (u_sum / 4.0).clamp(0.0, 255.0) as u8;
            uv_plane[uv + 1] = (v_sum / 4.0).clamp(0.0, 255.0) as u8;
        }
    }
    let mut out = Vec::with_capacity(width * height * 3 / 2);
    out.extend_from_slice(&y_plane);
    out.extend_from_slice(&uv_plane);
    out
}

unsafe impl Send for HardwareH264 {}

pub struct HardwareH264 {
    transform: IMFTransform,
    width: usize,
    height: usize,
    time: i64,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
    config_sent: bool,
}

pub fn available() -> bool {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        if MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).is_err() {
            return false;
        }
        let input = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: MFVideoFormat_NV12,
        };
        let output = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: MFVideoFormat_H264,
        };
        let mut activates: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut count = 0u32;
        let ok = MFTEnumEx(
            MFT_CATEGORY_VIDEO_ENCODER,
            MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER,
            Some(&input),
            Some(&output),
            &mut activates,
            &mut count,
        )
        .is_ok();
        let _ = MFShutdown();
        ok && count > 0
    }
}

impl HardwareH264 {
    pub fn new(width: usize, height: usize) -> Option<Self> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).ok()?;
            let input = MFT_REGISTER_TYPE_INFO {
                guidMajorType: MFMediaType_Video,
                guidSubtype: MFVideoFormat_NV12,
            };
            let output = MFT_REGISTER_TYPE_INFO {
                guidMajorType: MFMediaType_Video,
                guidSubtype: MFVideoFormat_H264,
            };
            let mut activates: *mut Option<IMFActivate> = std::ptr::null_mut();
            let mut count = 0u32;
            MFTEnumEx(
                MFT_CATEGORY_VIDEO_ENCODER,
                MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER,
                Some(&input),
                Some(&output),
                &mut activates,
                &mut count,
            )
            .ok()?;
            if count == 0 || activates.is_null() {
                eprintln!("hw_h264: no hardware MFT");
                return None;
            }
            let list = std::slice::from_raw_parts(activates, count as usize);
            let activate = list[0].as_ref()?;
            let transform: IMFTransform = match activate.ActivateObject() {
                Ok(t) => t,
                Err(e) => { eprintln!("hw_h264: activate: {e}"); return None; }
            };

            if let Ok(attrs) = transform.GetAttributes() {
                let _ = attrs.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1);
            }
            let out_type = transform
                .GetOutputAvailableType(0, 0)
                .unwrap_or_else(|_| MFCreateMediaType().ok().unwrap());

            let _ = out_type.SetUINT32(&MF_MT_AVG_BITRATE, 4_000_000);
            let _ = out_type.SetUINT32(
                &MF_MT_INTERLACE_MODE,
                MFVideoInterlace_Progressive.0 as u32,
            );
            let _ = out_type
                .SetUINT64(&MF_MT_FRAME_SIZE, ((width as u64) << 32) | height as u64);
            let _ = out_type.SetUINT64(&MF_MT_FRAME_RATE, 15u64 << 32);
            let _ = out_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, 1u64 << 32);
            let _ = out_type.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_Base.0 as u32);
            eprintln!(
                "hw_h264: out size={:?}",
                out_type.GetUINT64(&MF_MT_FRAME_SIZE)
            );
            if let Err(e) = transform.SetOutputType(0, &out_type, 0) {
                eprintln!("hw_h264: set output type: {e}");
                return None;
            }
            let in_type = match transform.GetInputAvailableType(0, 0) {
                Ok(t) => {
                    let g = t.GetGUID(&MF_MT_SUBTYPE).unwrap_or_default();
                    eprintln!(
                        "hw_h264: input subtype {:?} size={:?}",
                        g,
                        t.GetUINT64(&MF_MT_FRAME_SIZE)
                    );
                    t
                }
                Err(e) => {
                    eprintln!("hw_h264: no input type: {e}");
                    MFCreateMediaType().ok().unwrap()
                }
            };

            let _ = in_type.SetUINT32(
                &MF_MT_INTERLACE_MODE,
                MFVideoInterlace_Progressive.0 as u32,
            );
            let _ = in_type.SetUINT64(&MF_MT_FRAME_SIZE, ((width as u64) << 32) | height as u64);
            let _ = in_type.SetUINT64(&MF_MT_FRAME_RATE, 15u64 << 32);
            let _ = in_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, 1u64 << 32);
            let _ = in_type.SetUINT32(&MF_MT_DEFAULT_STRIDE, width as u32);
            let _ = in_type.SetUINT32(&MF_MT_FIXED_SIZE_SAMPLES, 1);
            let _ = in_type.SetUINT32(&MF_MT_SAMPLE_SIZE, (width * height * 3 / 2) as u32);
            if let Err(e) = transform.SetInputType(0, &in_type, 0) {
                eprintln!("hw_h264: set input type: {e}"); return None;
            }
            if let Err(e) = transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0) {
                eprintln!("hw_h264: begin streaming: {e}"); return None;
            }
            transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)
                .ok()?;
            Some(Self {
                transform,
                width,
                height,
                time: 0,
                sps: None,
                pps: None,
                config_sent: false,
            })
        }
    }

    pub fn encode(
        &mut self,
        rgb: &[u8],
    ) -> Result<(Vec<u8>, bool, Option<Vec<u8>>, Option<String>), String> {
        unsafe {
            let nv12 = rgb_to_nv12(rgb, self.width, self.height);
            let input_buffer =
                MFCreateMemoryBuffer(nv12.len() as u32).map_err(|e| e.to_string())?;
            let mut ptr = std::ptr::null_mut();
            input_buffer
                .Lock(&mut ptr, None, None)
                .map_err(|e| e.to_string())?;
            std::ptr::copy_nonoverlapping(nv12.as_ptr(), ptr, nv12.len());
            input_buffer.SetCurrentLength(nv12.len() as u32).map_err(|e| e.to_string())?;
            input_buffer.Unlock().map_err(|e| e.to_string())?;
            let sample = MFCreateSample().map_err(|e| e.to_string())?;
            sample.AddBuffer(&input_buffer).map_err(|e| e.to_string())?;
            let duration = 10_000_000 / 15;
            sample.SetSampleTime(self.time).map_err(|e| e.to_string())?;
            sample
                .SetSampleDuration(duration)
                .map_err(|e| e.to_string())?;
            self.time += duration;
            self.transform
                .ProcessInput(0, &sample, 0)
                .map_err(|e| e.to_string())?;

            let mut all = Vec::new();
            let mut keyframe = false;
            loop {
                let out_buffer =
                    MFCreateMemoryBuffer(4 * 1024 * 1024).map_err(|e| e.to_string())?;
                let out_sample = MFCreateSample().map_err(|e| e.to_string())?;
                out_sample.AddBuffer(&out_buffer).map_err(|e| e.to_string())?;
                let out = MFT_OUTPUT_DATA_BUFFER {
                    dwStreamID: 0,
                    pSample: core::mem::ManuallyDrop::new(Some(out_sample.clone())),
                    dwStatus: 0,
                    pEvents: core::mem::ManuallyDrop::new(None),
                };
                let mut outputs = [out];
                let mut status = 0u32;
                let result = self
                    .transform
                    .ProcessOutput(0, &mut outputs, &mut status);
                if let Err(err) = result {
                    let code = err.code().0 as u32;
                    if code == MF_E_TRANSFORM_NEED_MORE_INPUT.0 as u32 {
                        break;
                    }
                    if code == MF_E_TRANSFORM_STREAM_CHANGE.0 as u32 {
                        break;
                    }
                    return Err(err.to_string());
                }
                if let Some(sample) = &*outputs[0].pSample {
                    if let Ok(buffer) = sample.ConvertToContiguousBuffer() {
                        let mut ptr = std::ptr::null_mut();
                        let mut len = 0u32;
                        if buffer.Lock(&mut ptr, None, Some(&mut len)).is_ok() {
                            if len > 0 {
                                let data = std::slice::from_raw_parts(ptr, len as usize);
                                all.extend_from_slice(data);
                            }
                            let _ = buffer.Unlock();
                        }
                    }
                    if let Ok(clean) = sample.GetUINT32(&MFSampleExtension_CleanPoint) {
                        if clean != 0 {
                            keyframe = true;
                        }
                    }
                }
                if all.len() > 4 * 1024 * 1024 {
                    break;
                }
            }
            if all.is_empty() {
                return Ok((Vec::new(), keyframe, None, None));
            }
            let nals = h264_nals_from_annexb(&all);
            let mut payload = Vec::new();
            for nal in nals {
                if nal.is_empty() {
                    continue;
                }
                let ty = nal[0] & 0x1f;
                if ty == 7 {
                    self.sps = Some(nal.clone());
                } else if ty == 8 {
                    self.pps = Some(nal.clone());
                }
                payload.extend_from_slice(&(nal.len() as u32).to_be_bytes());
                payload.extend_from_slice(&nal);
            }
            if self.sps.is_none() || self.pps.is_none() {
                return Ok((payload, keyframe, None, None));
            }
            let mut avcc = None;
            let mut codec = None;
            if keyframe || !self.config_sent {
                let sps = self.sps.as_ref().unwrap();
                let pps = self.pps.as_ref().unwrap();
                avcc = Some(crate::build_avcc(sps, pps));
                codec = Some(format!(
                    "avc1.{:02X}{:02X}{:02X}",
                    sps.get(1).copied().unwrap_or(0x42),
                    sps.get(2).copied().unwrap_or(0xE0),
                    sps.get(3).copied().unwrap_or(0x1E)
                ));
                self.config_sent = true;
            }
            Ok((payload, keyframe, avcc, codec))
        }
    }
}
