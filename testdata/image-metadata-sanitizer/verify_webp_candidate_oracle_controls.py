#!/usr/bin/env python3
"""Sensitivity controls proving published WebP candidate observations fail closed."""
import copy
import importlib.util
import json
import sys
from pathlib import Path


def load(path):
    spec = importlib.util.spec_from_file_location("webp_candidate", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def rejected(observer, result, mutate):
    changed = copy.deepcopy(result)
    mutate(changed)
    try:
        observer.assert_candidate_observations(changed)
    except AssertionError:
        return
    raise AssertionError("candidate oracle accepted a promised-observation mutation")


if __name__ == "__main__":
    report_path, observer_path = map(Path, sys.argv[1:])
    observer = load(observer_path)
    report = json.loads(report_path.read_text())
    result = report["fixtures"][0]
    observer.assert_candidate_observations(result)
    mutations = (
        lambda value: value.update(candidate_structural_error="container drift"),
        lambda value: value.update(candidate_decode_error="decode drift"),
        lambda value: value["diagnostics"].update(stdout="unexpected"),
        lambda value: value["diagnostics"].update(stderr="warning"),
        lambda value: value["candidate"]["metadata"].update(EXIF="retained"),
        lambda value: value["candidate"]["metadata"].update(ICCP="changed"),
        lambda value: value["candidate"].update(flags=value["source"]["flags"]),
        lambda value: value["candidate"].update(canvas=[1, 1]),
        lambda value: value["candidate"].update(animation=None),
        lambda value: value["candidate"]["animation"].update(background_bgra="00000000"),
        lambda value: value["candidate"]["frames"][0]["rect"].__setitem__(2, 1),
        lambda value: value["candidate"]["frames"][0].update(blend=True),
        lambda value: value["candidate"]["frames"][0].update(dispose_to_background=True),
        lambda value: value["candidate"]["frames"][0].update(duration_ms=1),
        lambda value: value["candidate"]["frames"][0]["subchunks"][0].__setitem__(1, "changed"),
        lambda value: value.update(compressed_payloads_identical=False),
        lambda value: value["candidate_decode"]["frames"].__setitem__(0, "changed"),
        lambda value: value.update(raw_canvases_equal=False),
        lambda value: value["candidate_decode"].update(timings=[1]),
        lambda value: value["candidate_decode"].update(loop=99),
        lambda value: value["candidate_decode"].update(orientation=6),
        lambda value: value.update(orientation_retained=True),
        lambda value: value["candidate_decode"].update(displayed=[[[24, 32], "wrong"]]),
        lambda value: value["candidate_decode"].update(icc=False),
        lambda value: value["candidate_decode"].update(xmp=True),
        lambda value: value["candidate_decode"].update(exif_tags=[274]),
    )
    for mutate in mutations:
        rejected(observer, result, mutate)
    static = next(item for item in report["fixtures"] if item["fixture"].startswith("static-"))
    observer.assert_candidate_observations(static)
    rejected(observer, static, lambda value: value["candidate"]["image_payloads"][0].__setitem__(1, "changed"))

    # Source admission must not return early after inspecting transparency:
    # frame count, encoded geometry, loop, and timing remain required proofs.
    decoded = copy.deepcopy(result["source_decode"])
    decoded["displayed"] = [(tuple(size), digest) for size, digest in decoded["displayed"]]
    observer.source_requirements(result["fixture"], result["source"], decoded)
    source_mutations = (
        lambda view, value: view["frames"].pop(),
        lambda view, value: value["frames"].pop(),
        lambda view, value: value.update(loop=99),
        lambda view, value: value.update(timings=[1, 2, 3]),
        lambda view, value: view["frames"][0]["rect"].__setitem__(2, 1),
    )
    for mutate in source_mutations:
        view, changed = copy.deepcopy(result["source"]), copy.deepcopy(decoded)
        mutate(view, changed)
        try:
            observer.source_requirements(result["fixture"], view, changed)
        except AssertionError:
            continue
        raise AssertionError("source oracle skipped frame/loop/timing/geometry validation")
    print(f"{len(mutations) + 1} candidate-observation and {len(source_mutations)} source-admission report mutations rejected")
