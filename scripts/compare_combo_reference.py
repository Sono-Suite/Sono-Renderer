"""Repeat the DisplayHolic combo pixel regression using the production compositor.

Requires Python with numpy, scipy and Pillow, Cargo, FFmpeg, and the local
TestingSuite reference. No analysis dependency is added to the application.
See docs/COMBO_GLOW_LIGHT_CALIBRATION.md for provenance and interpretation.
"""

import argparse
import json
import math
from pathlib import Path
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "target/combo-calibration"
sys.path.insert(0, str(OUT / "python-libs"))
import numpy as np
from PIL import Image, ImageDraw
from scipy.ndimage import shift, binary_dilation


def records():
    # Beat positions verified against the DISPLAYHOLIC chart and video. The
    # writer rounds seconds to six places, exactly as PED's Go %f output does.
    result = []
    for combo, beat in [(2, 2.5), (3, 3), (4, 3.5), (6, 4),
                        (19, 7.5), (32, 11.5), (88, 20), (112, 24)]:
        event = 516 + round(beat * 60 / 110, 6) * 60
        offsets = [1, 3, 6, 7, 8, 9, 10, 11, 12, 13, 14] if combo < 10 else [8, 10, 12, 14]
        for offset in offsets:
            frame = math.ceil(event) + offset
            result.append(dict(id=f"c{combo}-f{frame}", combo=combo,
                               frame=frame, event_frame=event,
                               age=frame-event, time=frame/60-5))
    return result


def extract(ffmpeg, reference, samples):
    directory = OUT / "reference"
    directory.mkdir(parents=True, exist_ok=True)
    frames = sorted({r["frame"] for r in samples})
    selection = "+".join(f"eq(n\\,{f})" for f in frames)
    subprocess.run([str(ffmpeg), "-v", "error", "-i", str(reference), "-vf",
                    f"select={selection},crop=550:340:1350:300", "-fps_mode", "vfr",
                    "-frames:v", str(len(frames)), "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
                   check=True, stdout=(OUT / "reference.rgb").open("wb"))
    decoded = np.fromfile(OUT / "reference.rgb", dtype=np.uint8).reshape(-1, 340, 550, 3)
    assert len(decoded) == len(frames)
    for r in samples:
        Image.fromarray(decoded[frames.index(r["frame"])]).save(directory / (r["id"] + ".png"))


def capture(samples):
    requests = []
    for r in samples:
        for suffix, settings in [("after", {}), ("no-effects", {"effects": False}),
                                 ("input", {"part": "pop-input"})]:
            requests.append(dict(id=r["id"]+"-"+suffix, combo=r["combo"],
                                 age=r["age"], time=r["time"], **settings))
    (OUT / "requests.json").write_text(json.dumps(requests, indent=2), encoding="utf8")
    subprocess.run(["cargo", "test", "--lib", "capture_combo_reference_calibration",
                    "--", "--ignored", "--nocapture"], cwd=ROOT, check=True)


def analyze(samples):
    def load(r, suffix):
        return np.asarray(Image.open(OUT / "captures" / (r["id"]+"-"+suffix+".png")), dtype=float)

    results = []
    for r in samples:
        ref = np.asarray(Image.open(OUT / "reference" / (r["id"]+".png")), dtype=float)
        # Fixed using inactive-filter frames ONLY, before filter fitting.
        registered = shift(ref, (-.158, -.122, 0), order=1, mode="constant")
        # Excludes the COMBO title, which is intentionally not part of capture.
        roi = (slice(105, 295), slice(175, 395) if r["combo"] < 10 else slice(60, 500))
        matte = load(r, "input")[:, :, 3] > 32
        halo = binary_dilation(matte, iterations=25) & ~binary_dilation(matte, iterations=3)
        entry = {**r, "metrics": {}}
        for suffix in ["before", "no-effects", "after"]:
            if not (OUT / "captures" / (r["id"]+"-"+suffix+".png")).exists():
                continue  # Historical baseline is optional on subsequent runs.
            image = load(r, suffix)
            if suffix == "after" and (r["age"] <= 8 or r["age"] >= 13.5):
                assert abs(image-load(r, "no-effects")).max() <= 1, r
            delta = image - registered
            entry["metrics"][suffix] = dict(
                mae=float(abs(delta[roi]).mean()), mse=float((delta[roi]**2).mean()),
                raw_mae=float(abs((image-ref)[roi]).mean()),
                core_mae=float(abs(delta[roi])[matte[roi]].mean()) if matte[roi].any() else None,
                halo_mae=float(abs(delta[roi])[halo[roi]].mean()) if halo[roi].any() else None,
            )
        results.append(entry)

    aggregates = {}
    for label, combos in [("fit", [2, 4]), ("single_holdout", [3, 6]),
                           ("multi_holdout", [19, 32, 88, 112])]:
        group = [r for r in results if r["combo"] in combos and 8 < r["age"] < 13.5]
        aggregates[label] = {}
        for suffix in ["before", "no-effects", "after"]:
            available = [r["metrics"][suffix] for r in group if suffix in r["metrics"]]
            if available:
                aggregates[label][suffix] = {k: float(np.mean([v[k] for v in available if v[k] is not None]))
                                             for k in available[0]}
        # Independent video measurements, not constants copied from the shader.
        assert aggregates[label]["after"]["mae"] < 3.5, aggregates[label]
        assert aggregates[label]["after"]["mse"] < 40, aggregates[label]
        assert aggregates[label]["after"]["mae"] < aggregates[label]["no-effects"]["mae"], aggregates[label]

    # Each active age must remain useful; an average cannot hide a failed digit.
    for r in results:
        assert r["metrics"]["after"]["mae"] < 4.5, r
    report = dict(registration=dict(dx=.122, dy=.158, fitted_on="inactive-filter frames"),
                  roi=dict(y=[105,295], single_x=[175,395], multi_x=[60,500]),
                  aggregates=aggregates, samples=results)
    (OUT / "comparison.json").write_text(json.dumps(report, indent=2), encoding="utf8")
    print(json.dumps(aggregates, indent=2))

    for combo in sorted({r["combo"] for r in samples}):
        group = [r for r in samples if r["combo"] == combo and 8 < r["age"] < 14]
        sheet = Image.new("RGB", (440*4, 215*len(group)))
        draw = ImageDraw.Draw(sheet)
        for row, r in enumerate(group):
            ref = np.asarray(Image.open(OUT / "reference" / (r["id"]+".png")), dtype=float)
            after = load(r, "after")
            baseline = "before" if (OUT / "captures" / (r["id"]+"-before.png")).exists() else "no-effects"
            for col, (name, image) in enumerate([
                ("reference", ref), (baseline, load(r, baseline)), ("after", after),
                ("raw difference x4", abs(after-ref)*4),
            ]):
                tile = Image.fromarray(image.clip(0,255).astype("uint8")).crop((60,105,500,295))
                sheet.paste(tile, (col*440, row*215+25))
                draw.text((col*440+4,row*215+4), f"{name} c{combo} f{r['frame']} age{r['age']:.3f}", fill="white")
        sheet.save(OUT / f"comparison-c{combo}.png")


def encode_sequence(ffmpeg, samples):
    """Measure a separate codec pass; never fit the filter to that codec."""
    group = [r for r in samples if r["combo"] == 2 and 8 < r["age"] < 15]
    frames = []
    for r in group:
        canvas = Image.new("RGB", (1920, 1080))
        canvas.paste(Image.open(OUT / "captures" / (r["id"]+"-after.png")), (1350, 300))
        frames.append(np.asarray(canvas))
    original = np.stack(frames)
    output = OUT / "calibrated-combo-2-ages8-14.mp4"
    subprocess.run([str(ffmpeg), "-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24",
                    "-s", "1920x1080", "-r", "60", "-i", "-", "-c:v", "libx264",
                    "-preset", "medium", "-crf", "18", "-pix_fmt", "yuv420p",
                    "-movflags", "+faststart", str(output)], input=original.tobytes(), check=True)
    decoded = subprocess.run([str(ffmpeg), "-v", "error", "-i", str(output), "-f", "rawvideo",
                              "-pix_fmt", "rgb24", "-"], capture_output=True, check=True).stdout
    decoded = np.frombuffer(decoded, dtype=np.uint8).reshape(-1, 1080, 1920, 3)
    assert len(decoded) == len(group)
    roi = (slice(None), slice(405,595), slice(1525,1745), slice(None))
    delta = decoded[roi].astype(float)-original[roi].astype(float)
    results = dict(frames=len(group), dimensions=[1920,1080], fps=60, crf=18,
                   incremental_mae=float(abs(delta).mean()), incremental_mse=float((delta**2).mean()))
    reference_errors = []
    for r, frame in zip(group, decoded):
        ref = np.asarray(Image.open(OUT / "reference" / (r["id"]+".png")), dtype=float)
        ref = shift(ref, (-.158,-.122,0), order=1, mode="constant")
        error = (frame[300:640,1350:1900].astype(float)-ref)[105:295,175:395]
        reference_errors.append(dict(frame=r["frame"], age=r["age"],
                                     mae=float(abs(error).mean()), mse=float((error**2).mean())))
    results["encoded_vs_reference"] = reference_errors
    (OUT / "encoding-metrics.json").write_text(json.dumps(results, indent=2), encoding="utf8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference", type=Path, default=ROOT / "TestingSuite/DisplayHolic Blank.mp4")
    parser.add_argument("--ffmpeg", type=Path, default=ROOT / "dependencies/ffmpeg/8.1/ffmpeg.exe")
    parser.add_argument("--extract", action="store_true")
    parser.add_argument("--capture", action="store_true")
    parser.add_argument("--encode", action="store_true")
    args = parser.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    samples = records()
    (OUT / "samples.json").write_text(json.dumps(samples, indent=2), encoding="utf8")
    if args.extract:
        extract(args.ffmpeg, args.reference, samples)
    if args.capture:
        capture(samples)
    analyze(samples)
    if args.encode:
        encode_sequence(args.ffmpeg, samples)


if __name__ == "__main__":
    main()
