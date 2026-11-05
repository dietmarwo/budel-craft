#!/usr/bin/env python3
"""Regression evidence for the supplied judge transcription."""
import json
from pathlib import Path
from tempfile import TemporaryDirectory

from reference_score import load_case, score


def report(rings, placements, rules=None, plane=None):
    data = {
        "case": "audit", "plane": plane or {"w": 100, "h": 100}, "rules": rules or {},
        "shapes": [{"id": str(i), "ring": ring} for i, ring in enumerate(rings)],
    }
    with TemporaryDirectory() as tmp:
        path = Path(tmp) / "case.json"
        path.write_text(json.dumps(data), encoding="utf-8")
        row, _ = score(load_case(path), {"case": "audit", "placements": placements})
    return row


def main():
    ring = [[0, 0], [10, 0], [10, 10], [0, 10]]
    a = {"id": "0"}
    b = {"id": "1"}
    corner = report([ring, ring], [a, dict(b, x=10, y=10)])
    assert corner["valid"] and corner["tightest"] == 0
    rolled = ring[2:] + ring[:2]
    coincident = report([rolled, rolled], [a, b], plane={"w": 10, "h": 10})
    assert coincident["valid"] and coincident["coverage"] == 200
    relaxed = report([ring, ring], [a, dict(b, x=11.9)], {"min_channel": 2, "tolerance": 0.2})
    assert relaxed["valid"] and relaxed["below_target"]["target"] == 1
    p = {"w": 100, "h": 100, "margin": 10}
    from_plane = report([ring], [dict(a, x=10, y=10)], plane=p)
    from_rules = report([ring], [dict(a, x=10, y=10)], {"margin": 10}, p)
    assert (from_plane["coverage_usable"], from_rules["coverage_usable"]) == (1, 1.56)
    rules = {"poses": "eight"}
    assert report([ring], [dict(a, x=20, y=20, rot=1e-7)], rules)["valid"]
    assert not report([ring], [dict(a, x=20, y=20, rot=-1e-7)], rules)["valid"]
    print("Reproduced corner contact, coincident 200% coverage, relaxed gap, margin and angle wrap.")


if __name__ == "__main__":
    main()
