#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.10"
# dependencies = ["pyvista", "numpy"]
# ///
"""Provenance-graph 3D viewer — a coverage diagnostic, not decoration.

Snapshots the live db (read-only URI open + sqlite backup into a temp copy;
never renders from the live file), lays out connected components with a small
force-directed pass, parks isolated nodes on a ring, and stamps the image with
the sha256 of the copy it rendered so every image is re-derivable.
"""
import argparse
import hashlib
import os
import sqlite3
import sys
import tempfile
from datetime import datetime, timezone

import numpy as np

DEFAULT_DB = os.path.expanduser("~/.local/share/neural-memory/store.db")

CLASS_COLORS = {
    "observed": "#2e7d32",
    "derivedDeterministically": "#1565c0",
    "humanDecision": "#f9a825",
    "agentInference": "#8e24aa",
    "externalClaim": "#795548",
}
KIND_COLORS = {  # muted, except contradicts
    "supersedes": "#9e9e9e",
    "derivedFrom": "#78909c",
    "supports": "#a5d6a7",
    "citesArtifact": "#b0bec5",
    "contradicts": "#d32f2f",
}


def snapshot(db_path):
    """Read-only open of the live db, backup into a temp copy, return (copy_path, sha256, utc_ts)."""
    if not os.path.isfile(db_path):
        sys.exit(f"{db_path}: no such database file")
    src = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    fd, copy_path = tempfile.mkstemp(prefix="graph3d-", suffix=".db")
    os.close(fd)
    dst = sqlite3.connect(copy_path)
    src.backup(dst)
    dst.close()
    src.close()
    h = hashlib.sha256()
    with open(copy_path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return copy_path, h.hexdigest(), datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def load(copy_path):
    """Read the temp copy. Returns (nodes: digest->class, edges: [(src,dst,kind)] drawn, dangling, edge_refd: set)."""
    con = sqlite3.connect(copy_path)
    nodes = dict(con.execute("SELECT record_digest, evidence_class FROM memories"))
    raw = con.execute("SELECT src_digest, dst_digest, edge_kind FROM provenance_edges").fetchall()
    con.close()
    edges = [(s, d, k) for s, d, k in raw if s in nodes and d in nodes]
    dangling = len(raw) - len(edges)
    edge_refd = {x for s, d, _ in raw for x in (s, d)}  # matches the SQL isolated definition
    return nodes, edges, dangling, edge_refd


def components(node_ids, edges):
    """Union-find. Returns list of components (lists of digests) among edge-touching nodes,
    plus the list of degree-0 (in drawn graph) digests."""
    parent = {n: n for n in node_ids}

    def find(a):
        while parent[a] != a:
            parent[a] = parent[parent[a]]
            a = parent[a]
        return a

    for s, d, _ in edges:
        parent[find(s)] = find(d)
    touched = {x for s, d, _ in edges for x in (s, d)}
    comps = {}
    for n in touched:
        comps.setdefault(find(n), []).append(n)
    loners = [n for n in node_ids if n not in touched]
    return list(comps.values()), loners


def fr_layout(n, edge_idx, rng, iters=80):
    """Small 3D Fruchterman-Reingold; n is tiny (<=259), O(n^2) per iter is fine."""
    if n == 1:
        return np.zeros((1, 3))
    pos = rng.uniform(-1, 1, (n, 3))
    k = (1.0 / n) ** (1.0 / 3.0)
    for it in range(iters):
        delta = pos[:, None, :] - pos[None, :, :]
        dist = np.linalg.norm(delta, axis=-1)
        np.fill_diagonal(dist, 1.0)
        disp = (delta / dist[..., None] * (k * k / dist)[..., None]).sum(axis=1)
        for a, b in edge_idx:
            d = pos[a] - pos[b]
            dn = np.linalg.norm(d) + 1e-9
            f = d / dn * (dn * dn / k)
            disp[a] -= f
            disp[b] += f
        ln = np.linalg.norm(disp, axis=1, keepdims=True)
        ln[ln < 1e-9] = 1e-9
        temp = 0.1 * (1 - it / iters) + 0.005
        pos += disp / ln * np.minimum(ln, temp)
    pos -= pos.mean(axis=0)
    r = np.linalg.norm(pos, axis=1).max() or 1.0
    return pos / r  # unit radius


def layout(nodes, edges):
    """Positions for every node: components force-laid then offset on a grid; loners on a ring."""
    rng = np.random.default_rng(42)
    comps, loners = components(list(nodes), edges)
    comps.sort(key=len, reverse=True)
    pos = {}
    spacing = 3.0
    ncols = max(1, int(np.ceil(np.sqrt(len(comps))))) if comps else 1
    for i, comp in enumerate(comps):
        idx = {n: j for j, n in enumerate(comp)}
        eidx = [(idx[s], idx[d]) for s, d, _ in edges if s in idx and d in idx]
        p = fr_layout(len(comp), eidx, rng) * (0.4 * spacing * min(1.0, 0.3 + len(comp) / 20))
        off = np.array([(i % ncols) * spacing, (i // ncols) * spacing, 0.0])
        for n, j in idx.items():
            pos[n] = p[j] + off
    # ring of isolated nodes, clearly apart: surrounds the component grid
    if loners:
        center = np.array([(ncols - 1) * spacing / 2, ((max(len(comps) - 1, 0)) // ncols) * spacing / 2, 0.0])
        grid_r = np.linalg.norm(center) + spacing if comps else 0.0
        ring_r = grid_r + 2 * spacing
        for j, n in enumerate(sorted(loners)):
            a = 2 * np.pi * j / len(loners)
            pos[n] = center + np.array([ring_r * np.cos(a), ring_r * np.sin(a), 0.0])
    return pos, len(comps), loners


def render(nodes, edges, dangling, edge_refd, sha, ts, screenshot=None):
    import pyvista as pv

    pos, n_comps, loners = layout(nodes, edges)
    n = len(nodes)
    e = len(edges)
    isolated = sorted(set(nodes) - edge_refd)  # same definition as the coverage SQL
    mean_deg = 2 * e / n if n else 0.0
    pct_iso = 100.0 * len(isolated) / n if n else 0.0

    pl = pv.Plotter(off_screen=screenshot is not None, window_size=(1600, 1100))
    pl.set_background("white")

    # nodes, one mesh per evidence class so the legend is categorical with counts
    legend = []
    by_class = {}
    for d, c in nodes.items():
        by_class.setdefault(c, []).append(pos[d])
    for cls in CLASS_COLORS:
        pts = by_class.get(cls)
        if not pts:
            continue
        label = f"{cls} ({len(pts)})"
        pl.add_mesh(pv.PolyData(np.array(pts)), color=CLASS_COLORS[cls],
                    render_points_as_spheres=True, point_size=14, label=label)
        legend.append([label, CLASS_COLORS[cls]])

    # edges, one mesh per kind; contradicts red + thick, others muted thin
    by_kind = {}
    for s, d, k in edges:
        by_kind.setdefault(k, []).append((pos[s], pos[d]))
    contradiction_pts = []
    for kind, segs in by_kind.items():
        pts = np.array([p for seg in segs for p in seg])
        lines = np.hstack([[2, 2 * i, 2 * i + 1] for i in range(len(segs))])
        width = 5 if kind == "contradicts" else 1.5
        pl.add_mesh(pv.PolyData(pts, lines=lines), color=KIND_COLORS.get(kind, "#cccccc"),
                    line_width=width, label=f"{kind} ({len(segs)})")
        legend.append([f"{kind} ({len(segs)})", KIND_COLORS.get(kind, "#cccccc")])
        if kind == "contradicts":
            contradiction_pts = pts

    # label only contradiction endpoints (short digests); 259 full labels would be noise
    if len(contradiction_pts):
        cdigests = [x for s, d, k in edges if k == "contradicts" for x in (s, d)]
        pl.add_point_labels(contradiction_pts, [c[:8] for c in cdigests],
                            font_size=11, text_color="#b71c1c", shape=None, always_visible=True)

    pl.add_legend(legend, bcolor="white", face=None, size=(0.22, 0.32), loc="upper right")
    pl.add_text(
        f"{n} nodes, {e} edges, mean degree {mean_deg:.2f}, {pct_iso:.0f}% isolated"
        " — provenance coverage, not decoration",
        position="upper_edge", font_size=12, color="black")
    pl.add_text(
        f"db-copy sha256 {sha}\n"
        f"copy: {n} memories, {e + dangling} edge rows ({e} drawn, {dangling} dangling), "
        f"snapshot {ts}",
        position="lower_left", font_size=7, color="black")

    if screenshot:
        pl.screenshot(screenshot)
        pl.close()
    else:
        pl.show()
    return {"nodes": n, "edges_drawn": e, "dangling": dangling,
            "isolated": len(isolated), "components": n_comps, "ring": len(loners)}


FIXTURE_SCHEMA = """
CREATE TABLE memories (record_digest TEXT PRIMARY KEY, claim TEXT,
  evidence_class TEXT, occurred_at TEXT);
CREATE TABLE provenance_edges (src_digest TEXT, dst_digest TEXT,
  edge_kind TEXT, created_at TEXT);
"""


def self_check():
    dig = lambda s: hashlib.sha256(s.encode()).hexdigest()
    a, b, c, d, e, f = (dig(x) for x in "abcdef")
    fd, db = tempfile.mkstemp(prefix="graph3d-fixture-", suffix=".db")
    os.close(fd)
    con = sqlite3.connect(db)
    con.executescript(FIXTURE_SCHEMA)
    rows = [(a, "claim a", "observed"), (b, "claim b", "agentInference"),
            (c, "claim c", "externalClaim"), (d, "claim d", "humanDecision"),
            (e, "claim e", "derivedDeterministically"), (f, "claim f", "observed")]
    con.executemany("INSERT INTO memories VALUES (?,?,?,'2026-01-01')", rows)
    con.executemany("INSERT INTO provenance_edges VALUES (?,?,?,'2026-01-01')",
                    [(a, b, "supports"), (b, c, "contradicts"), (d, e, "derivedFrom")])
    con.commit()
    con.close()
    try:
        copy, sha, ts = snapshot(db)
        try:
            nodes, edges, dangling, edge_refd = load(copy)
            fd2, png = tempfile.mkstemp(prefix="graph3d-fixture-", suffix=".png")
            os.close(fd2)
            stats = render(nodes, edges, dangling, edge_refd, sha, ts, screenshot=png)
            # hand-computed: components {a,b,c} and {d,e} = 2; only f isolated = 1
            assert stats["components"] == 2, stats
            assert stats["isolated"] == 1, stats
            assert stats["ring"] == 1, stats
            assert stats["nodes"] == 6 and stats["edges_drawn"] == 3 and stats["dangling"] == 0, stats
            assert os.path.getsize(png) > 0
            os.unlink(png)
        finally:
            os.unlink(copy)
    finally:
        os.unlink(db)
    print("self-check ok")


def main():
    ap = argparse.ArgumentParser(description="3D provenance-coverage diagnostic")
    ap.add_argument("--db", default=DEFAULT_DB)
    ap.add_argument("--screenshot", metavar="OUT.png")
    ap.add_argument("--self-check", action="store_true")
    args = ap.parse_args()

    if args.self_check:
        self_check()
        return

    copy, sha, ts = snapshot(args.db)
    try:
        nodes, edges, dangling, edge_refd = load(copy)
        stats = render(nodes, edges, dangling, edge_refd, sha, ts, screenshot=args.screenshot)
    finally:
        os.unlink(copy)
    print(f"nodes={stats['nodes']} edges_drawn={stats['edges_drawn']} "
          f"dangling={stats['dangling']} isolated={stats['isolated']} "
          f"components={stats['components']} sha256={sha}", file=sys.stderr)


if __name__ == "__main__":
    main()
