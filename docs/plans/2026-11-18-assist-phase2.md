# Assist Phase 2: Quality and Adaptation

Implemented on top of the WebSocket transport:

## Dirty-region capture

- Captures the virtual desktop through GDI.
- Builds a 160x90 thumbnail and compares it with the previous frame.
- Sends only the changed bounding rectangle as a partial JPEG.
- Sends a full keyframe every ~3 seconds or after screen-size changes.
- Frame header version 2:

```text
magic "IDVF"
u16 version (2)
u16 flags (bit0 keyframe)
u32 sequence
u64 timestamp ms
u16 screen width
u16 screen height
u16 rect x
u16 rect y
u16 rect width
u16 rect height
JPEG bytes
```

## Adaptive quality / FPS

Browser sends `ack` after drawing a frame:

```json
{"type":"ack","seq":12,"rtt":35}
```

Agent starts at quality 60 / 100 ms interval:

- RTT > 250 ms: lower quality and FPS;
- RTT < 90 ms: raise quality and FPS gradually;
- send queue keeps at most two frames; when full the current frame is dropped.

Browser shows current FPS and RTT next to the mode tag.

## Ping/pong

Browser sends `{"type":"ping","t":...}` every 2 seconds; Agent replies
`{"type":"pong","t":...}`. The browser derives RTT from the round trip.

## Multi-monitor / DPI

- Capture uses `SM_XVIRTUALSCREEN` / `SM_YVIRTUALSCREEN` / virtual width/height.
- Mouse coordinates are mapped against the virtual desktop.
- The assist helper calls `SetProcessDPIAware()` before capture.

## Not included

- DXGI Desktop Duplication is optional and not enabled in this phase; GDI is
  still used as the capture path.
- H.264/VP8 and audio are phase 3.
