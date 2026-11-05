#!/usr/bin/env python3
"""score.py — the judge of the budel packing assignment.

    python3 score.py cases/small.json reference/small.answer.json
    python3 score.py cases/small.json my.answer.json --against reference/small.answer.json
    python3 score.py cases/small.json my.answer.json --render my-small.png
    python3 score.py cases/small.json my.answer.json --validate-only
    python3 score.py cases/small.json my.answer.json --json out.json
    python3 score.py --reproduce

It decides whether an arrangement obeys the hard rules and how dense it is. It needs nothing
but Python 3.9+, NumPy and Pillow — no network, no project, no lab.

Exit codes: 0 = valid, 1 = invalid, 2 = the files could not be read or do not belong together.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import sys
import time

import numpy as np

try:
    from PIL import Image, ImageDraw
except Exception:
    Image = None
    ImageDraw = None

RASTER = 1.0
SUPERSAMPLE = 2
BORDER_BAND = 0.10
EIGHT = [0.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0]
POSE_EPS = 1e-6

VIOLATION_KINDS = [
    "unknown-shape",
    "duplicate-shape",
    "forbidden-pose",
    "outside-plane",
    "overlap",
    "too-close",
    "must-seat-all",
]


class LoadError(Exception):
    """The files could not be read or do not belong together (exit code 2)."""


def load_json(path):
    try:
        with open(path, "r", encoding="utf-8") as fh:
            return json.load(fh)
    except FileNotFoundError:
        raise LoadError("file not found: %s" % path)
    except json.JSONDecodeError as exc:
        raise LoadError("%s is not valid JSON: %s" % (path, exc))


def load_case(path):
    case = load_json(path)
    for key in ("case", "plane", "rules", "shapes"):
        if key not in case:
            raise LoadError("%s is not a case file (missing '%s')" % (path, key))
    shapes = {}
    for sh in case["shapes"]:
        if "id" not in sh or "ring" not in sh:
            raise LoadError("%s: a shape without id or ring" % path)
        if sh["id"] in shapes:
            raise LoadError("%s: shape id '%s' appears twice" % (path, sh["id"]))
        sh = dict(sh)
        sh["ring"] = np.asarray(sh["ring"], dtype=float)
        sh["holes"] = [np.asarray(h, dtype=float) for h in sh.get("holes", [])]
        if sh.get("envelope"):
            sh["envelope"] = np.asarray(sh["envelope"], dtype=float)
        shapes[sh["id"]] = sh
    case["shapes_by_id"] = shapes
    return case


def load_answer(path, case):
    ans = load_json(path)
    if "placements" not in ans:
        raise LoadError("%s is not an answer file (no 'placements')" % path)
    if ans.get("case") and case and ans["case"] != case["case"]:
        raise LoadError(
            "%s answers case '%s', but the case file is '%s'" % (path, ans["case"], case["case"])
        )
    return ans


def transform(ring, x, y, rot, mirror):
    pts = ring.copy()
    if mirror:
        pts = pts * np.array([-1.0, 1.0])
    a = math.radians(rot)
    co, si = math.cos(a), math.sin(a)
    rotm = np.array([[co, -si], [si, co]])
    pts = pts @ rotm.T
    pts[:, 0] += x
    pts[:, 1] += y
    return pts


def polygon_area(ring):
    x, y = ring[:, 0], ring[:, 1]
    return 0.5 * abs(float(np.dot(x, np.roll(y, -1)) - np.dot(np.roll(x, -1), y)))


def bbox(ring):
    return (float(ring[:, 0].min()), float(ring[:, 1].min()), float(ring[:, 0].max()), float(ring[:, 1].max()))


def seg_seg_min_distance(a, b):
    p = a
    q = np.roll(a, -1, axis=0)
    r = b
    s = np.roll(b, -1, axis=0)
    d1 = q - p
    d2 = s - r
    diff = p[:, None, :] - r[None, :, :]
    a11 = np.einsum("ij,ij->i", d1, d1)[:, None]
    a22 = np.einsum("ij,ij->i", d2, d2)[None, :]
    a12 = d1 @ d2.T
    b1 = np.einsum("nmj,nj->nm", np.broadcast_to(diff, (len(p), len(r), 2)), d1)
    b2 = np.einsum("nmj,mj->nm", np.broadcast_to(diff, (len(p), len(r), 2)), d2)
    denom = a11 * a22 - a12 * a12
    parallel = np.abs(denom) < 1e-12
    with np.errstate(invalid="ignore", divide="ignore"):
        t1 = (a12 * b2 - a22 * b1) / denom
        t2 = (a11 * b2 - a12 * b1) / denom
    t1 = np.where(parallel, 0.0, t1)
    t2 = np.where(parallel, 0.0, t2)
    crossing = bool(np.any((~parallel) & (t1 > 1e-9) & (t1 < 1 - 1e-9) & (t2 > 1e-9) & (t2 < 1 - 1e-9)))
    t1c = np.clip(t1, 0.0, 1.0)
    t2c = np.clip(t2, 0.0, 1.0)
    def dist_at(t1v, t2v):
        pa = p[:, None, :] + d1[:, None, :] * t1v[..., None]
        pb = r[None, :, :] + d2[None, :, :] * t2v[..., None]
        return np.linalg.norm(pa - pb, axis=2)
    best = dist_at(t1c, t2c)
    for (tfix, which) in ((t1c, 1), (t2c, 2)):
        if which == 1:
            pa = p[:, None, :] + d1[:, None, :] * tfix[..., None]
            tt = np.einsum("nmj,mj->nm", pa - r[None, :, :], d2) / np.where(a22 == 0, 1.0, a22)
            tt = np.clip(tt, 0.0, 1.0)
            best = np.minimum(best, dist_at(tfix, tt))
        else:
            pb = r[None, :, :] + d2[None, :, :] * tfix[..., None]
            tt = np.einsum("nmj,nj->nm", pb - p[:, None, :], d1) / np.where(a11 == 0, 1.0, a11)
            tt = np.clip(tt, 0.0, 1.0)
            best = np.minimum(best, dist_at(tt, tfix))
    return float(best.min()), crossing


def point_in_polygon(pt, ring):
    x, y = pt
    xs, ys = ring[:, 0], ring[:, 1]
    xs2, ys2 = np.roll(xs, -1), np.roll(ys, -1)
    cond = ((ys > y) != (ys2 > y))
    with np.errstate(invalid="ignore", divide="ignore"):
        xint = (xs2 - xs) * (y - ys) / np.where(ys2 - ys == 0, np.nan, ys2 - ys) + xs
    hits = np.count_nonzero(cond & (x < xint))
    return hits % 2 == 1


def placed_rings(case, answer, violations):
    shapes = case["shapes_by_id"]
    rules = case["rules"]
    poses = rules.get("poses", "free")
    mirror_allowed = bool(rules.get("mirror_allowed", True))
    body = rules.get("body", "rest")
    seen = set()
    out = []
    for pl in answer["placements"]:
        sid = pl.get("id")
        if sid not in shapes:
            violations.append({"rule": "unknown-shape", "ids": [sid], "measured": None, "limit": None})
            continue
        if sid in seen:
            violations.append({"rule": "duplicate-shape", "ids": [sid], "measured": None, "limit": None})
            continue
        seen.add(sid)
        rot = float(pl.get("rot", 0.0)) % 360.0
        mirror = bool(pl.get("mirror", False))
        if poses == "eight" and not any(abs(rot - p) < POSE_EPS for p in EIGHT):
            violations.append({"rule": "forbidden-pose", "ids": [sid], "measured": rot, "limit": "one of %s" % EIGHT})
        if mirror and not mirror_allowed:
            violations.append({"rule": "forbidden-pose", "ids": [sid], "measured": "mirror", "limit": "mirror not allowed"})
        sh = shapes[sid]
        ring = sh["envelope"] if (body == "envelope" and sh.get("envelope") is not None) else sh["ring"]
        pts = transform(np.asarray(ring, dtype=float), float(pl.get("x", 0.0)), float(pl.get("y", 0.0)), rot, mirror)
        holes = [transform(h, float(pl.get("x", 0.0)), float(pl.get("y", 0.0)), rot, mirror) for h in sh["holes"]]
        out.append({"id": sid, "ring": pts, "holes": holes, "bbox": bbox(pts)})
    return out


def check_plane(case, placed, violations):
    plane = case["plane"]
    m = float(case["rules"].get("margin", plane.get("margin", 0.0)))
    x0, y0 = m, m
    x1, y1 = float(plane["w"]) - m, float(plane["h"]) - m
    for item in placed:
        bx0, by0, bx1, by1 = item["bbox"]
        if bx0 < x0 - 1e-9 or by0 < y0 - 1e-9 or bx1 > x1 + 1e-9 or by1 > y1 + 1e-9:
            worst = max(x0 - bx0, y0 - by0, bx1 - x1, by1 - y1)
            violations.append({"rule":"outside-plane","ids":[item["id"]],"measured":round(float(worst),3),"limit":"usable plane %.1f..%.1f x %.1f..%.1f"%(x0,x1,y0,y1)})


def neighbour_pairs(placed, reach):
    if not placed:
        return []
    cell = max(32.0, reach * 2.0, np.median([i["bbox"][2] - i["bbox"][0] for i in placed]))
    buckets = {}
    for idx, item in enumerate(placed):
        bx0, by0, bx1, by1 = item["bbox"]
        for cx in range(int(math.floor((bx0 - reach) / cell)), int(math.floor((bx1 + reach) / cell)) + 1):
            for cy in range(int(math.floor((by0 - reach) / cell)), int(math.floor((by1 + reach) / cell)) + 1):
                buckets.setdefault((cx, cy), []).append(idx)
    pairs = set()
    for ids in buckets.values():
        for i in range(len(ids)):
            for j in range(i + 1, len(ids)):
                a, b = (ids[i], ids[j]) if ids[i] < ids[j] else (ids[j], ids[i])
                pairs.add((a, b))
    keep = []
    for a, b in sorted(pairs):
        ax0, ay0, ax1, ay1 = placed[a]["bbox"]
        bx0, by0, bx1, by1 = placed[b]["bbox"]
        dx = max(0.0, max(bx0 - ax1, ax0 - bx1))
        dy = max(0.0, max(by0 - ay1, ay0 - by1))
        if math.hypot(dx, dy) <= reach:
            keep.append((a, b))
    return keep


def check_distances(case, placed, violations):
    rules = case["rules"]
    channel = float(rules.get("min_channel", 0.0))
    tol = float(rules.get("tolerance", 0.0))
    reach = max(channel * 4.0, channel + 1.0, 8.0)
    pairs = neighbour_pairs(placed, reach)
    tightest = math.inf
    below = {"target": 0, "half": 0, "quarter": 0}
    for a, b in pairs:
        pa, pb = placed[a], placed[b]
        dist, crossing = seg_seg_min_distance(pa["ring"], pb["ring"])
        inside = (not crossing) and (point_in_polygon(pa["ring"][0], pb["ring"]) or point_in_polygon(pb["ring"][0], pa["ring"]))
        if crossing or inside:
            violations.append({"rule":"overlap","ids":[pa["id"],pb["id"]],"measured":0.0,"limit":channel})
            tightest = 0.0
            below["target"] += 1
            below["half"] += 1
            below["quarter"] += 1
            continue
        tightest = min(tightest, dist)
        if dist < channel:
            below["target"] += 1
            if dist < channel / 2:
                below["half"] += 1
            if dist < channel / 4:
                below["quarter"] += 1
        if dist < channel - tol:
            violations.append({"rule":"too-close","ids":[pa["id"],pb["id"]],"measured":round(dist,3),"limit":channel})
    return (None if tightest is math.inf else round(tightest, 3)), below, len(pairs)


def raster_masks(case, placed):
    if Image is None:
        raise LoadError("Pillow is required for the measures; install it with: pip install pillow")
    plane = case["plane"]
    paper_holes = (case.get("rules") or {}).get("holes_are", "paper") == "paper"
    k = RASTER * SUPERSAMPLE
    w = max(1, int(round(float(plane["w"]) * k)))
    h = max(1, int(round(float(plane["h"]) * k)))
    img = Image.new("L", (w, h), 0)
    dr = ImageDraw.Draw(img)
    for item in placed:
        ring = [(float(x) * k, float(y) * k) for x, y in item["ring"]]
        if len(ring) >= 3:
            dr.polygon(ring, fill=255)
        if paper_holes:
            for hole in item["holes"]:
                hr = [(float(x) * k, float(y) * k) for x, y in hole]
                if len(hr) >= 3:
                    dr.polygon(hr, fill=0)
    fine = np.array(img) > 127
    if SUPERSAMPLE == 1:
        return fine.astype(np.float32)
    hh = (fine.shape[0] // SUPERSAMPLE) * SUPERSAMPLE
    ww = (fine.shape[1] // SUPERSAMPLE) * SUPERSAMPLE
    block = fine[:hh, :ww].reshape(hh // SUPERSAMPLE, SUPERSAMPLE, ww // SUPERSAMPLE, SUPERSAMPLE)
    return block.mean(axis=(1, 3), dtype=np.float32)


def ink_area(case, placed):
    paper_holes = (case.get("rules") or {}).get("holes_are", "paper") == "paper"
    total = 0.0
    for item in placed:
        total += polygon_area(item["ring"])
        if paper_holes:
            for hole in item["holes"]:
                if len(hole) >= 3:
                    total -= polygon_area(hole)
    return total


def measure(case, placed):
    plane = case["plane"]
    m = float(case["rules"].get("margin", 0.0))
    ink = raster_masks(case, placed)
    h, w = ink.shape
    area = ink_area(case, placed)
    coverage = 100.0 * area / (float(plane["w"]) * float(plane["h"]))
    mx0 = int(round(m * RASTER))
    my0 = int(round(m * RASTER))
    mx1 = w - mx0
    my1 = h - my0
    usable = ink[my0:my1, mx0:mx1]
    free = usable < 0.5
    free_total = int(free.sum())
    band = max(1, int(round(BORDER_BAND * min(usable.shape))))
    border = np.zeros_like(free)
    border[:band, :] = True
    border[-band:, :] = True
    border[:, :band] = True
    border[:, -band:] = True
    border_free = int((free & border).sum())
    border_share = (100.0 * border_free / free_total) if free_total else 0.0
    usable_area = max(1.0, (float(plane["w"]) - 2 * m) * (float(plane["h"]) - 2 * m))
    coverage_usable = 100.0 * area / usable_area
    return {"coverage":round(coverage,2),"ink_area":round(area,1),"coverage_usable":round(coverage_usable,2),"border_free_share":round(border_share,1)}


def score(case, answer, validate_only=False):
    started = time.time()
    violations = []
    placed = placed_rings(case, answer, violations)
    check_plane(case, placed, violations)
    tightest, below, pairs = check_distances(case, placed, violations)
    if case["rules"].get("must_seat_all") and len(placed) < len(case["shapes"]):
        violations.append({"rule":"must-seat-all","ids":[],"measured":len(placed),"limit":len(case["shapes"])})
    classes = {}
    for sh in case["shapes"]:
        k = sh.get("size_class")
        if k is None:
            continue
        classes.setdefault(float(k), {"available":0,"seated":0})["available"] += 1
    seated_ids = {item["id"] for item in placed}
    for sh in case["shapes"]:
        k = sh.get("size_class")
        if k is not None and sh["id"] in seated_ids:
            classes[float(k)]["seated"] += 1
    row = {"case":case["case"],"author":answer.get("author",""),"declared":{"runtime_s":answer.get("runtime_s"),"budget":answer.get("budget"),"machine":answer.get("machine")},"regime":(case.get("mix") or {}).get("regime"),"seated_by_class":[{"size":k,"seated":v["seated"],"available":v["available"]} for k,v in sorted(classes.items(),key=lambda kv:-kv[0])],"valid":not violations,"violations":violations,"shapes":len(case["shapes"]),"seated":len(placed),"tightest":tightest,"below_target":below,"pairs_checked":pairs}
    if not validate_only:
        row.update(measure(case, placed))
    row["runtime_s"] = round(time.time() - started, 2)
    return row, placed


def render(case, placed, out_path, detail=None, scale=0.5):
    if Image is None:
        raise LoadError("Pillow is required for --render")
    plane = case["plane"]
    m = float(case["rules"].get("margin", 0.0))
    ss = 3
    w = max(1, int(round(float(plane["w"]) * scale)))
    h = max(1, int(round(float(plane["h"]) * scale)))
    img = Image.new("RGB", (w * ss, h * ss), (240, 240, 213))
    dr = ImageDraw.Draw(img)
    k = scale * ss
    for item in placed:
        ring = [(float(x) * k, float(y) * k) for x, y in item["ring"]]
        if len(ring) >= 3:
            dr.polygon(ring, fill=(20, 20, 20))
        if (case.get("rules") or {}).get("holes_are", "paper") == "paper":
            for hole in item["holes"]:
                hr = [(float(x) * k, float(y) * k) for x, y in hole]
                if len(hr) >= 3:
                    dr.polygon(hr, fill=(240, 240, 213))
        else:
            for hole in item["holes"]:
                hr = [(float(x) * k, float(y) * k) for x, y in hole]
                if len(hr) >= 3:
                    dr.polygon(hr, fill=(150, 90, 70))
    dr.rectangle([m * k, m * k, (float(plane["w"]) - m) * k, (float(plane["h"]) - m) * k],
                 outline=(190, 60, 60), width=max(1, ss))
    img = img.resize((w, h), Image.LANCZOS)
    if detail:
        x0, y0, x1, y1 = [float(v) for v in detail]
        crop = img.crop((int(x0 * scale), int(y0 * scale), int(x1 * scale), int(y1 * scale)))
        crop = crop.resize((crop.width * 2, crop.height * 2), Image.LANCZOS)
        crop.save(out_path)
    else:
        img.save(out_path)
    return out_path


def fmt_row(row, case_runtime=None):
    lines = []
    if row["valid"]:
        lines.append("  VALID")
    else:
        lines.append("  INVALID — %d violation%s" % (len(row["violations"]), "" if len(row["violations"]) == 1 else "s"))
        for i, v in enumerate(row["violations"][:20], 1):
            ids = " / ".join(str(x) for x in v["ids"]) if v["ids"] else "—"
            lines.append("  %2d. %-14s %-18s measured %-10s limit %s"
                         % (i, v["rule"], ids, v["measured"], v["limit"]))
        if len(row["violations"]) > 20:
            lines.append("      … and %d more" % (len(row["violations"]) - 20))
    lines.append("")
    lines.append("  seated              %6d / %d" % (row["seated"], row["shapes"]))
    by_class = row.get("seated_by_class") or []
    if by_class:
        shown = by_class[:12]
        cells = ["%g: %d/%d" % (c["size"], c["seated"], c["available"]) for c in shown]
        tail = "" if len(by_class) <= 12 else "  … and %d more classes" % (len(by_class) - 12)
        lines.append("  per size class      %s%s" % (" · ".join(cells), tail))
    if "coverage" in row:
        lines.append("  ink coverage        %6.2f %%   (of the whole plane, exact polygon area)"
                     % row["coverage"])
        lines.append("  … of the usable plane %4.2f %%" % row["coverage_usable"])
    if row["tightest"] is not None:
        lines.append("  tightest pair       %6.2f units" % row["tightest"])
    b = row["below_target"]
    lines.append("  pairs below target  %6d        (below 1/2: %d · below 1/4: %d)"
                 % (b["target"], b["half"], b["quarter"]))
    if "border_free_share" in row:
        lines.append("  free space at border %5.1f %%" % row["border_free_share"])
    lines.append("  scored in %.2f s (%d neighbour pairs measured)" % (row["runtime_s"], row["pairs_checked"]))

    d = row.get("declared") or {}
    rt = (case_runtime or {})
    if d.get("runtime_s") is not None:
        budget_name = d.get("budget")
        limit = (rt.get("budgets") or {}).get(budget_name) if budget_name else None
        verdict = ""
        if limit:
            verdict = " → kept" if float(d["runtime_s"]) <= limit else " → BUDGET EXCEEDED"
        per = 1000.0 * float(d["runtime_s"]) / max(1, row["seated"])
        ours = (1000.0 * rt["lab_seconds"] / max(1, row["shapes"])) if rt.get("lab_seconds") else None
        lines.append("  declared run        %6.1f s%s%s" % (
            float(d["runtime_s"]),
            ("  ·  budget %s (%d s)" % (budget_name, limit)) if limit else
            ("  ·  budget %s" % budget_name if budget_name else "  ·  no budget named"),
            verdict))
        lines.append("  %sper figure           %5.2f ms%s" % (
            "", per, ("   (ours: %.2f ms)" % ours) if ours else ""))
        if d.get("machine"):
            lines.append("  measured on         %s   (their statement, not ours)" % d["machine"])
    elif rt:
        lines.append("  declared run        not declared — this version asks for runtime_s and budget")
    return "\n".join(lines)


def header(case):
    r = case["rules"]
    mix = case.get("mix") or {}
    first = ("case %s · %d shapes · plane %g x %g · margin %g · channel %g · body %s · poses %s"
             % (case["case"], len(case["shapes"]), case["plane"]["w"], case["plane"]["h"],
                r.get("margin", 0), r.get("min_channel", 0), r.get("body", "rest"),
                r.get("poses", "free")))
    first += " · holes %s" % r.get("holes_are", "paper")
    if not mix:
        return first
    steps = " · ".join("d %s x%d" % (k, v) for k, v in (mix.get("growth_steps") or {}).items())
    return first + ("\n%s regime · %d size class(es) · %s · %g %% of the figures stand at d >= 0"
                    % (mix.get("regime", "—"), len(mix.get("size_classes") or {}),
                       steps or "growth steps unknown", mix.get("hard_share", 0.0)))


def curve(case, answer_paths, case_path):
    """Density over time: one row per answer, sorted by budget, our own run as a row."""
    base = os.path.dirname(os.path.dirname(os.path.abspath(case_path)))
    ref = os.path.join(base, "reference", "%s.answer.json" % case["case"])
    paths = list(answer_paths)
    if os.path.exists(ref) and os.path.abspath(ref) not in [os.path.abspath(x) for x in paths]:
        paths.append(ref)

    rt = (case.get("rules") or {}).get("runtime") or {}
    order = {"interactive": 0, "patient": 1, "background": 2}
    rows = []
    for path in paths:
        ans = load_answer(path, case)
        r, _ = score(case, ans)
        declared = ans.get("runtime_s")
        is_ours = os.path.abspath(path) == os.path.abspath(ref)
        if declared is None and is_ours:
            declared = rt.get("lab_seconds")
        rows.append({
            "name": "our own engine" if is_ours else os.path.basename(path),
            "budget": ans.get("budget") or ("our build" if is_ours else "—"),
            "seconds": declared,
            "coverage": r.get("coverage"),
            "seated": r["seated"],
            "valid": r["valid"],
        })
    rows.sort(key=lambda r: (order.get(r["budget"], 9), r["seconds"] or 0))

    print(header(case))
    print()
    print("  %-34s %-12s %9s %11s %9s %9s" % ("answer", "budget", "seconds", "coverage", "seated", "gain"))
    last = None
    for r in rows:
        gain = "" if last is None or r["coverage"] is None else "%+.2f" % (r["coverage"] - last)
        print("  %-34s %-12s %9s %10.2f %% %9d %9s%s"
              % (r["name"][:34], r["budget"],
                 ("%.1f" % r["seconds"]) if r["seconds"] is not None else "—",
                 r["coverage"], r["seated"], gain, "" if r["valid"] else "   INVALID"))
        last = r["coverage"]
    print()
    print("  the seconds are what each answer declares; only our own row was measured here.")
    return 0 if all(r["valid"] for r in rows) else 1


def reproduce(root):
    """Score every reference answer of the package and compare with the published table."""
    pub_path = os.path.join(root, "reference", "scores.json")
    published = load_json(pub_path)
    ok = True
    for entry in published["scores"]:
        case = load_case(os.path.join(root, entry["case_file"]))
        answer = load_answer(os.path.join(root, entry["answer_file"]), case)
        row, _ = score(case, answer)
        same = (row["valid"] == entry["valid"]
                and row["seated"] == entry["seated"]
                and abs(row["coverage"] - entry["coverage"]) < 0.01
                and (row["tightest"] is None or abs(row["tightest"] - entry["tightest"]) < 0.01))
        print("%-18s published %6.2f %%  ·  measured %6.2f %%  ·  %s"
              % (entry["case"], entry["coverage"], row["coverage"], "OK" if same else "DIFFERS"))
        ok = ok and same
    print("\nreproduced: %s" % ("YES" if ok else "NO"))
    return 0 if ok else 1


def main(argv=None):
    ap = argparse.ArgumentParser(description="Score a budel packing answer.")
    ap.add_argument("case", nargs="?", help="a case file, e.g. cases/small.json")
    ap.add_argument("answer", nargs="?", help="an answer file")
    ap.add_argument("more", nargs="*", help="further answers of the same case (with --curve)")
    ap.add_argument("--curve", action="store_true",
                    help="density over time: several answers of one case, sorted by budget")
    ap.add_argument("--against", help="a second answer for the same case (usually ours)")
    ap.add_argument("--render", metavar="PNG", help="draw the arrangement")
    ap.add_argument("--detail", help="x0,y0,x1,y1 — render a crop instead of the whole plane")
    ap.add_argument("--validate-only", action="store_true", help="rules only, no measures")
    ap.add_argument("--json", metavar="FILE", help="write the score as JSON")
    ap.add_argument("--quiet", action="store_true")
    ap.add_argument("--reproduce", action="store_true", help="re-measure the package's own table")
    args = ap.parse_args(argv)

    try:
        if args.reproduce:
            return reproduce(os.path.dirname(os.path.abspath(__file__)))

        if not args.case or not args.answer:
            ap.print_help()
            return 2

        case = load_case(args.case)
        if args.curve:
            return curve(case, [args.answer] + list(args.more), args.case)
        answer = load_answer(args.answer, case)
        row, placed = score(case, answer, validate_only=args.validate_only)

        if not args.quiet:
            print(header(case))
            print()
            print(fmt_row(row, (case.get("rules") or {}).get("runtime")))

        if args.against:
            other = load_answer(args.against, case)
            orow, _ = score(case, other, validate_only=args.validate_only)
            row["against"] = {k: orow[k] for k in ("author", "valid", "seated", "coverage",
                                                   "tightest", "seated_by_class")
                              if k in orow}
            if not args.quiet:
                print()
                print("  against %s (%s):" % (args.against, orow.get("author") or "reference"))
                if "coverage" in row and "coverage" in orow:
                    print("      coverage  %7.2f %%  vs %7.2f %%   →  %+.2f points"
                          % (row["coverage"], orow["coverage"], row["coverage"] - orow["coverage"]))
                print("      seated    %7d    vs %7d" % (row["seated"], orow["seated"]))
                mine = {c["size"]: c for c in (row.get("seated_by_class") or [])}
                theirs = {c["size"]: c for c in (orow.get("seated_by_class") or [])}
                if len(mine) > 1:
                    for size in sorted(mine, reverse=True)[:12]:
                        a = mine[size]
                        b = theirs.get(size, {"seated": 0})
                        print("        class %-8g %5d    vs %7d   (of %d)"
                              % (size, a["seated"], b["seated"], a["available"]))
                if row["tightest"] is not None and orow["tightest"] is not None:
                    print("      tightest  %7.2f    vs %7.2f" % (row["tightest"], orow["tightest"]))
                print("      valid     %7s    vs %7s" % (row["valid"], orow["valid"]))

        if args.render:
            detail = args.detail.split(",") if args.detail else None
            path = render(case, placed, args.render, detail=detail)
            if not args.quiet:
                print("\n  drawn: %s" % path)

        if args.json:
            with open(args.json, "w", encoding="utf-8") as fh:
                json.dump(row, fh, indent=2, sort_keys=True)
            if not args.quiet:
                print("  written: %s" % args.json)

        return 0 if row["valid"] else 1

    except LoadError as exc:
        print("error: %s" % exc, file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
