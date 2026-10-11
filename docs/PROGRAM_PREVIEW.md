# Program Preview

The desktop GUI has a still-frame Program Preview for inspecting the current
render configuration through the production Watch and frame-rendering path.

## Use

1. Select the engine, resources, level, optional music/MV, and gameplay UI
   profile in the existing Project / inputs and settings tabs.
2. Open **Program preview**.
3. Seek within the configured start/duration window, or use Start, End, and
   frame-step controls. Left and right arrow keys step one configured frame
   when a text field does not have keyboard focus.
4. Choose **Render frame**. Navigation buttons request a new frame directly.

The worker captures a `RenderConfig` snapshot and calls its existing
`render_frame` API. A result is shown only if its request generation and
configuration still match the current GUI state. A newer seek supersedes a
stale in-flight result. The monitor caps its longest dimension at 1920 pixels;
the export configuration is unchanged. Final video still uses the existing
render job, queue, and export path.

## Current limits

This is a still-frame monitor, not sequential playback. Each seek creates a
fresh renderer preview and advances the Watch runtime to that frame, so long
seek times can be expensive. Preview work is not cancellable and the window
waits for it to finish before closing. The time slider uses the configured
start and duration even when Whole Chart is selected; Whole Chart's inferred
end is only known during export.

This Program Preview remains separate from the general-purpose **Presentation
Studio**. Program Preview seeks the production Watch renderer; Presentation
Studio imports local media into a standalone composition and uses FFmpeg for
still-frame previews and export. See [Presentation Studio](PRESENTATION_STUDIO.md)
for its composition workflow and limits.
