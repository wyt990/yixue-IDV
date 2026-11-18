# Assist WebSocket Transport (Phase 1)

The remote-assist path now has a multiplexed WebSocket transport in addition to
the older HTTP polling fallback.

## Endpoint

```text
/api/v1/assist/sessions/{session_id}/ws?role=viewer&token=...
/api/v1/assist/sessions/{session_id}/ws?role=agent&mac=...
```

Center keeps one in-memory broker per session:

- one Agent socket;
- one or more viewer sockets;
- viewer messages are forwarded to the Agent;
- Agent messages are broadcast to viewers.

## Video packet

Agent sends binary WebSocket messages:

```text
offset  size  field
0       4     magic "IDVF"
4       2     version (1)
6       2     flags (bit0 keyframe)
8       4     frame sequence
12      8     timestamp ms
20      ...   JPEG payload
```

The browser parses the header with DataView and renders the JPEG through
`createImageBitmap` + canvas. Frames are taken at about 10 fps; when the send
queue is full the current frame is dropped instead of building latency.

## Input and clipboard

Viewer sends JSON text messages:

```json
{"type":"input","event":{"type":"mouse_move","x":0.5,"y":0.5}}
{"type":"clipboard_set","text":"..."}
{"type":"clipboard_get"}
```

Agent sends clipboard changes as:

```json
{"type":"clipboard","text":"..."}
```

Mouse moves are coalesced in the browser before sending.

## Files

The existing HTTP file browser still works: the Agent WebSocket loop polls the
old assist command queue for `fs_*` requests. Large file payloads can also use
the chunked WebSocket messages:

```json
{"type":"file_get","id":"...","path":"C:\\..."}
{"type":"file_begin","id":"...","name":"a.txt","size":1234}
{"type":"file_chunk","id":"...","offset":0,"data":"<hex>"}
{"type":"file_end","id":"..."}
{"type":"file_put_start","id":"...","path":"C:\\...","size":1234}
{"type":"file_ack","id":"..."}
{"type":"file_error","id":"...","message":"..."}
```

## Fallback

If the WebSocket cannot be established, the Agent returns to the old HTTP
helper path and the previous frame/command endpoints remain available.
