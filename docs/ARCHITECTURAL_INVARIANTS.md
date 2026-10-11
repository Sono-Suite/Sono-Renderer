# Architectural invariants

These are behavior contracts, not implementation preferences. Changes may
refactor internals, but must preserve these semantics or update the contract
with new evidence and compatibility handling.

## Runtime and rendering

1. **Sono VM defines Watch semantics.** Sono-GCC is optional acceleration for
   eligible scalar regions. Runtime cuts and unsupported operations stay on
   the VM path; native failure falls back without claiming new semantics.
2. **There is one authoritative Watch lifecycle.** `WatchRuntime` owns entity
   callbacks and scheduling; `WatchVm` evaluates Watch operations. Export,
   previews, diagnostics, compatibility probes, and regression use those
   implementations instead of adding another interpreter or playback loop.
3. **A frame owns its snapshot.** Once Watch advances, downstream render,
   presentation, and audio work consume the completed frame's data rather
   than reading mutable runtime state from a later frame.
4. **Presentation cannot rewrite gameplay.** HUD profiles, particles,
   watermark, and media compositing consume runtime results. Presentation
   styling may change pixels, but does not change Watch state or its semantics.
5. **The LevelData media clock is fixed:** `media = Watch + bgmOffset`, and
   `Watch = media - bgmOffset`. Export-relative time subtracts the aligned
   Watch start; audio sample time derives from that output-relative interval.
6. **The direct render path stays simple.** Engine + LevelData + resources and
   optional music can render without a project, profile catalog, queue,
   history, or notification service.

## Authoring and execution

7. **`formats::LevelData` is canonical.** Chart Studio edits it, the renderer
   loads it, and export serializes it. Authoring schemas add optional labels
   and controls; they are not required to reopen or render exported LevelData.
8. **A project is editable input state.** A profile is reusable configuration.
   A job captures resolved configuration at submission. History describes an
   attempt; a manifest describes a reproducible input/output set. Re-rendering
   creates a new job and new history record.
9. **One project envelope owns project persistence.** Chart data is a
   versioned extension of `RenderProject`; do not add a parallel chart or video
   project schema without an explicit migration design.
10. **Workspace previews are production consumers.** Chart Studio and Program
    Preview call the renderer/runtime; preview-only code must not simulate
    Watch or presentation semantics.

## Caches, persistence, and side effects

11. **Caches are disposable.** Cache loss may cause recompilation or derived
    asset rebuilding, but must not erase projects, charts, profiles, or render
    semantics. Cache identity and validation protect correctness before hit
    rate.
12. **Persistent formats have an owner and version.** Their owning subsystem
    validates, migrates, and saves them. Atomic writers stage data before
    replacing a valid prior file; corruption in optional stores must not
    prevent unrelated direct rendering from starting.
13. **Secrets stay out of portable artifacts.** Notification credentials are
    resolved from secure/current settings and are not copied into project,
    history, profile exports, or render manifests.
14. **Auxiliary failures do not falsify render outcomes.** Notification
    delivery cannot fail a render. A partial output is not reported as a
    completed render; history records the actual terminal result.
15. **Disabled attribution stays disabled as configured.** The watermark
    master switch gates both fixed and moving stages, and the attribution
    literal is not made a user-editable string.

See [the architecture map](ARCHITECTURE.md) for owners, persistence formats,
worker lifetimes, and intentional couplings.
