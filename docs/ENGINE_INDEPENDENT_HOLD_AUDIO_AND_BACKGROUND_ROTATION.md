# Engine Independent Hold Audio and Background Rotation

## Findings

### Choose Your Fighter hold audio

The engine explicitly requests the hold sound. In the supplied
`Choose_Your_Fighter_1791912757.json.gz` fixture, three `Connector` entities
reference the fake hold head `hold_36` and its `AnchorNote` tail `hold_37`.
The Watch audio trace records three `PlayLoopedScheduled` calls and three
matching `StopLoopedScheduled` calls for the engine-bound `#HOLD` clip. The
connector callbacks issue those calls; fake-note classification is not used
to decide whether audio plays.

The intervals captured from Watch are:

| Start (Watch seconds) | Stop (Watch seconds) | Handle |
| ---: | ---: | ---: |
| 2.7857142857 | 3.0000000000 | 0 |
| 9.4285714286 | 9.5357142857 | 1 |
| 16.7142857143 | 16.8214285714 | 2 |

Each stop targets the handle returned for its start, each interval is
positive, and advancing or directly seeking the runtime does not add a second
copy of the schedule. This behavior is intentional engine output. No VM,
audio-scheduler, fake-note, or chart-specific production change was made.

The short real-media check exported the 2.7–3.2s interval with BGM disabled
and SFX enabled. The produced 320×180 MP4 contains H.264 video and 44.1kHz
stereo AAC; FFmpeg measured the decoded audio at −22.0dB mean and −3.0dB peak.
The matching `--no-sfx` control reported `SFX mixed: false`.

### Runtime background geometry

Watch exposes the engine's current Runtime Background as a four-corner quad
in memory block 1004. The renderer already uses this Watch-produced quad for
the native engine background. The issue was that selected MV footage instead
used its own fixed fit quad.

At 10.0s in the full DISPLAYHOLIC fixture, the Watch trace attributes writes
to block 1004 to `StaticStage.UpdateSequential` (entity 3944). The resulting
quad is rotated while Runtime Skin Transform remains the identity matrix:

```text
BL (-2.3423431, -2.2013168)
TL (-2.2013168,  2.3423431)
TR ( 2.3423431,  2.2013168)
BR ( 2.2013168, -2.3423431)
```

This is engine-produced geometry, not a renderer interpretation of a
`CameraChange` archetype. The opt-in maps that affine Watch transform from the
selected Sonolus background's initial quad onto the selected MV's quad. It
uses the Watch snapshot for each requested frame, so arbitrary seeks do not
depend on earlier rendered frames. When the affine update has no rotation,
the MV quad is left unchanged; this avoids turning a scale-only setup update
into a visible change. When a rotation is present, the engine's affine
transform, including its pivot/translation and scale, is transferred.

The native Sonolus background continues using its existing Watch quad with
the option either way. With an MV selected, the option defaults off and only
changes the MV background. Gameplay Draws, HUD, and watermark remain in the
screen-space compositor and are not rotated. Non-affine/perspective Runtime
Background quads are rejected with an error instead of being approximated.

## Configuration and integration

- `RenderConfig.apply_engine_background_rotation` is serde-defaulted to
  `false`, preserving older configuration files.
- The GUI exposes **Apply engine-driven background rotation** in the MV
  background controls while an MV is selected and enabled.
- A project stores an optional per-project override and resolves it back into
  the effective render configuration.
- `render-video` exposes the opt-in flag
  `--apply-engine-background-rotation`; omission leaves it disabled.
- Preview frame rendering and MP4 export both carry the setting into the same
  `render_prepared_frame` implementation.

No engine name, chart title, or archetype check was added to production code.

## Real-media comparison

All comparisons used the rebuilt Windows release executable, CPU rendering,
640×360 output, 2fps, and a 10.0–11.0s range. FFmpeg decoded output frames to
raw video for per-frame MD5 comparison.

| Fixture | Rotation setting | Decoded frame hashes (10.0s, 10.5s) |
| --- | --- | --- |
| DISPLAYHOLIC full | Off | `8fc332024a43bdc937edbf63e7f056ce`, `176d6b63c5e90d55f1681d7c89ffca35` |
| DISPLAYHOLIC full | On | `710b32b6c7e90c777bc4869cacd899be`, `8a0ac7f1777cdfb677ee46ce8df22201` |
| DISPLAYHOLIC Cut Ver | Off | `b24a69704a80ddb65ae644ef21347f47`, `e8595ef9c5dd12f16c3df008d6b1fff9` |
| DISPLAYHOLIC Cut Ver | On | `b24a69704a80ddb65ae644ef21347f47`, `e8595ef9c5dd12f16c3df008d6b1fff9` |
| Choose Your Fighter | Off | `848bb33681a51e97c24109cafa450591`, `5ead82a94649b5fcf6e837c625fc4a10` |
| Choose Your Fighter | On | `848bb33681a51e97c24109cafa450591`, `5ead82a94649b5fcf6e837c625fc4a10` |

The decoded DISPLAYHOLIC full frames were visually inspected side by side:
the MV/background image changes orientation, while gameplay geometry and the
watermark remain fixed. The full-chart Watch Draw hashes also match between
the enabled and disabled renders, confirming the Watch gameplay result itself
is unchanged.

The specifically requested `DisplayHolic Cut Ver.json.gz` is not a positive
rotation fixture at the checked timestamp: it contains no `CameraChange`
entities, and its 10.0s Runtime Background quad is axis aligned. It therefore
matches byte-for-byte with the option on and off. The full DISPLAYHOLIC chart
in the same fixture directory was used as the positive camera-effect sample.
This report does not claim parity for camera behavior that the Cut Ver chart
does not emit.

## Validation

- `cargo fmt --check`: passed.
- `cargo check --offline`: passed.
- `cargo test --offline background_transform_tests`: 3 passed, including
  rotation/pivot mapping, non-affine rejection, and scale/translation-only
  pass-through.
- `cargo test --offline hold`: 2 passed: the Choose Your Fighter trace test
  and the canonical Next RUSH hold regression.
- `cargo test --offline looped_scheduled`: 4 passed for independent loop
  handles, callback-shared IDs, and matching stops.
- `cargo test --offline mixes_play_and_loop_requests`: 1 passed.
- Project round-trip, render-config round-trip/default, and CLI opt-in parse
  tests: passed.
- `cargo build --offline --release --bin renderer`: passed. The Windows GUI
  executable is `target/release/renderer.exe`; it also exposes the CLI.
- Real H.264/AAC exports and decoded-frame comparisons: passed as recorded
  above. Export used the local shared FFmpeg/FFprobe installation.
- The GUI control's wiring was build-checked and its effective setting was
  exercised through project/config tests and CLI export. A separate
  interactive GUI session was not automated.

The Windows build reports that `rc.exe` was unavailable, so executable
version metadata was not embedded. The sandbox also denied writes to the
user AppData performance/diagnostic directories and native notifications
reported an environment error; these warnings did not prevent media exports.
The packaged installer and clean-machine FFmpeg discovery were not tested.

## Compatibility limits

Rotation following requires selected native Sonolus background geometry as a
reference and an affine Runtime Background quad. An engine that expresses its
camera only through Draw-specific transformations, a different undocumented
channel, or perspective geometry will not be inferred by this option. No
archetype-specific camera adapter was added. The control setting is currently
scoped to selected MV footage; native engine backgrounds already consume the
Watch quad directly.
