Reference Source Provenance

## Optional archival inspection material

The following exact repository-relative directories are retired, optional
research/development material and may be absent from an active checkout:

- `TestingSuite/android-sdk-live/`
- `TestingSuite/Sonolus Inspection/`
- `sono-apk/` (at the repository root)

They are not renderer resources, build prerequisites, test fixtures required by
the current Rust tests, or packaging inputs. Android SDK/emulator tools and
extracted APK files are not required to run Sono-Renderer. Historical research
links into these trees require restoring the archive to inspect that evidence.
Do not introduce production dependencies on their tools or extracted assets.

This archival scope does not include `TestingSuite/Next Sekai Engine/`,
`TestingSuite/Sono-Overlay/`, or `TestingSuite/ui_ref/`. Real-engine and reference
tests may still require those independent local fixtures. In particular, a HUD
unit test includes Sono-Overlay Lua at compile time; archiving the inspection
trees does not make every other fixture optional.

See [the dependency audit and isolated validation](docs/ARCHIVAL_DEPENDENCY_AUDIT.md).

## Provenance and usage

All third-party source trees present in the local reference environment
were obtained from their respective authors with permission to provide
the source directly to Codex for use while developing Sono-Renderer.

These sources may therefore be inspected and compared freely during
development.

Their presence does not imply:
- that they are open-source licensed;
- that Sono-Renderer may redistribute them;
- that their behavior is authoritative Sonolus behavior;
- or that their authors endorse Sono-Renderer.

Where redistribution rights are not separately established, reference
source must remain excluded from distributed Sono-Renderer source and
artifacts.

Semantic conclusions should distinguish:
1. official Sonolus documentation,
2. actual Sonolus observations,
3. Sono-Renderer behavior,
4. third-party reference implementation behavior.
