# Symphonia Seek API Notes

## Key Points

1. **FormatReader::seek()** method signature:
   ```rust
   fn seek(&mut self, mode: SeekMode, to: SeekTo) -> Result<SeekedTo>;
   ```

2. **SeekMode** options:
   - `Coarse`: Best-effort, may be before or after requested position
   - `Accurate`: Always seeks to position BEFORE requested position

3. **SeekTo** options:
   - `Time { time: Time, track_id: Option<u32> }`: Seek to time in seconds
   - `TimeStamp { ts: TimeStamp, track_id: u32 }`: Seek to timestamp in track's timebase

4. **Important**: After seeking, all Decoders must be reset!

5. **Process**:
   - Create FormatReader from MediaSourceStream
   - Call seek() to get to position
   - Reset decoder
   - Continue reading packets from new position

## Implementation Strategy

We need to:
1. Keep FormatReader alive (not just Sink)
2. Store current decoder state
3. On seek command:
   - Call format_reader.seek()
   - Reset/recreate decoder
   - Continue playback from new position
4. Use symphonia's Time struct to convert seconds to seek position
