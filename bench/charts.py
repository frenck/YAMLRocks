"""Render the comparison benchmarks into the docs assets, themed to the docs.

Run with ``just charts`` (which builds a *release* extension first, syncs the
``charts`` group, and runs this). The release build matters: a debug build is
several times slower, so charting one would understate YAMLRocks against the
release wheels of the other libraries.

It runs [`compare.py`](./compare.py) once and writes two SVGs into
``docs/public/benchmarks/``: load throughput and dump throughput across the
field. The charts are dark cards (so they read on both the light and dark docs
themes) using the docs' brand colours and font. Numbers are machine-dependent,
like the tables; regenerate on the machine you want to quote.
"""

from __future__ import annotations

import importlib.metadata
from pathlib import Path
from typing import Any

import matplotlib as mpl

import compare

mpl.use("Agg")

import matplotlib.pyplot as plt

# The docs' palette: the #100F15 base, the #506468 -> #24fefd brand gradient, Inter.
BG = "#13151c"
TEXT = "#e6edf3"
MUTED = "#9aa3ad"
GRID = "#2a2e38"
CYAN = "#24fefd"  # YAMLRocks (the brand highlight)
GREY = "#5b626d"  # a competitor library
GREY2 = "#41464f"  # a second competitor on a multi-competitor chart (e.g. PyYAML)
COMPETITOR_COLORS = [GREY, GREY2]

OUT = Path(__file__).resolve().parent.parent / "docs" / "public" / "benchmarks"

plt.rcParams.update(
    {
        "font.family": "sans-serif",
        "font.sans-serif": ["Inter", "Inter Variable", "DejaVu Sans"],
        "svg.fonttype": "none",  # keep text as text so the page font (Inter) applies
        "figure.facecolor": BG,
        "axes.facecolor": BG,
        "savefig.facecolor": BG,
        "text.color": TEXT,
        "axes.labelcolor": MUTED,
        "xtick.color": MUTED,
        "ytick.color": TEXT,
        "axes.edgecolor": GRID,
    }
)


# The PyPI distribution behind each `compare.py` label, so a chart can print the
# exact version measured (benchmark numbers are version-specific). YAMLRocks is
# deliberately omitted: it is built here from an untagged working tree, so a
# version string would be misleading.
_DIST = {
    "yaml_rs": "yaml-rs",
    "fast-yaml": "fastyaml-rs",
    "ryaml": "ryaml",
    "py-yaml12": "py-yaml12",
    "yamlium": "yamlium",
    "strictyaml": "strictyaml",
    "oyaml": "oyaml",
    "ruamel.yaml": "ruamel.yaml",
    "PyYAML (C)": "PyYAML",
    "PyYAML (pure)": "PyYAML",
}


def _labelled(label: str, display: str | None = None) -> str:
    """`display` (or `label`) with the installed version appended, when known."""
    name = display if display is not None else label
    dist = _DIST.get(label)
    if dist:
        try:
            return f"{name} {importlib.metadata.version(dist)}"
        except importlib.metadata.PackageNotFoundError:
            pass
    return name


def _fmt(us: float) -> str:
    """A compact duration label: microseconds, then milliseconds, then seconds."""
    if us < 1000:
        return f"{us:.0f} µs"
    if us < 1_000_000:
        return f"{us / 1000:.1f} ms"
    return f"{us / 1_000_000:.2f} s"


def _style(ax: Any, title: str, xlabel: str) -> None:
    """Apply the shared dark-card styling to an axis."""
    ax.set_title(title, color=TEXT, fontsize=13, fontweight="bold", pad=14, loc="left")
    ax.set_xlabel(xlabel, fontsize=9)
    for side in ("top", "right", "left"):
        ax.spines[side].set_visible(False)
    ax.spines["bottom"].set_color(GRID)
    ax.tick_params(length=0)
    ax.set_axisbelow(True)
    ax.xaxis.grid(visible=True, color=GRID, linewidth=0.8)


def _chart(rows: list[dict[str, Any]], key: str, title: str, filename: str) -> None:
    """A horizontal, log-scale bar chart of ``key`` (microseconds), fastest first.

    YAMLRocks is drawn in the brand cyan; every other library is a neutral grey.
    """
    rows = sorted(
        (row for row in rows if row.get(key) is not None), key=lambda row: row[key]
    )
    labels = [_labelled(row["label"]) for row in rows]
    values = [row[key] for row in rows]
    colors = [CYAN if row["label"] == "YAMLRocks" else GREY for row in rows]

    fig, ax = plt.subplots(figsize=(8.2, 0.5 * len(rows) + 1.4))
    base = range(len(rows))
    ax.barh(list(base), values, color=colors, height=0.72, zorder=3)

    ax.set_xscale("log")
    for y, value in zip(base, values, strict=True):
        ax.text(
            value * 1.15,
            y,
            _fmt(value),
            va="center",
            ha="left",
            color=MUTED,
            fontsize=7.5,
        )

    ax.set_yticks(list(base))
    ax.set_yticklabels(labels)
    ax.invert_yaxis()  # fastest at the top
    ax.set_xlim(right=max(values) * 3.2)
    _style(
        ax, title, "time to process the payload set once (log scale, lower is faster)"
    )

    path = OUT / filename
    fig.savefig(path, format="svg", bbox_inches="tight")
    plt.close(fig)
    # matplotlib leaves trailing whitespace on some SVG lines, which the repo's
    # trailing-whitespace hook would strip on commit; strip it here so a freshly
    # generated chart is already clean and does not fail that hook in CI.
    cleaned = "\n".join(line.rstrip() for line in path.read_text().splitlines())
    path.write_text(cleaned + "\n")


# Each page's chart: the output filename and the competitor(s) to draw against
# YAMLRocks, as `(label in compare.py, display name)`. Order mirrors the sidebar.
# Most are a single rival; PyYAML shows both its C loader and its pure-Python
# loader, since the page discusses both. The "Nx faster" headline is measured
# against the first competitor listed (for PyYAML, the C loader, the fair path).
_HEAD_TO_HEAD = [
    (
        "vs-pyyaml.svg",
        [("PyYAML (C)", "PyYAML (C loader)"), ("PyYAML (pure)", "PyYAML (pure)")],
    ),
    ("vs-ruamel.svg", [("ruamel.yaml", "ruamel.yaml")]),
    ("vs-yaml-rs.svg", [("yaml_rs", "yaml-rs")]),
    ("vs-fast-yaml.svg", [("fast-yaml", "fast-yaml")]),
    ("vs-ryaml.svg", [("ryaml", "ryaml")]),
    ("vs-py-yaml12.svg", [("py-yaml12", "py-yaml12")]),
    ("vs-yamlium.svg", [("yamlium", "yamlium")]),
    ("vs-strictyaml.svg", [("strictyaml", "strictyaml")]),
    ("vs-oyaml.svg", [("oyaml", "oyaml")]),
]

# The two operations each head-to-head chart shows, top to bottom.
_METRICS = [("Reading (loads)", "load_us"), ("Writing (dumps)", "dump_us")]


def _head_to_head(
    rows: list[dict[str, Any]],
    competitors: list[tuple[str, str]],
    filename: str,
) -> None:
    """A head-to-head time chart of YAMLRocks against one or more competitors, on
    reading and writing. Each bar is the time to process the payload set once,
    labelled with that time; YAMLRocks is the brand cyan, competitors grey.

    Shorter is faster, the convention every benchmark chart uses. YAMLRocks' bar
    leads with how many times faster it is (against the first competitor listed)
    and its time; each competitor's bar carries its time. On a linear axis a much
    slower library reads as a much longer bar, which is the honest picture.
    `competitors` is a list of `(label in compare.py, display name)`.
    """
    by_label = {row["label"]: row for row in rows}
    us = by_label.get("YAMLRocks")
    them = [(by_label.get(label), display) for label, display in competitors]
    if us is None or any(row is None for row, _ in them):
        missing = [
            c for c, (row, _) in zip(competitors, them, strict=True) if row is None
        ]
        print(f"skipping {filename}: not measured ({missing})")
        return

    # Only the operations YAMLRocks and every competitor support (strictyaml has
    # no dumper). `primary` is the first competitor, the one the multiple is
    # measured against (for PyYAML, the C loader).
    metrics = [
        (name, key)
        for name, key in _METRICS
        if us.get(key) is not None and all(row.get(key) is not None for row, _ in them)
    ]
    primary = them[0][0]
    bars = [(us, CYAN, True)] + [
        (row, COMPETITOR_COLORS[j % len(COMPETITOR_COLORS)], False)
        for j, (row, _) in enumerate(them)
    ]
    n = len(bars)
    longest = max(row[key] for row, _, _ in bars for _, key in metrics)

    def _mult(ratio: float) -> str:
        return f"{ratio:.1f}" if ratio < 10 else f"{ratio:.0f}"

    # With one rival, YAMLRocks carries the "Nx faster" headline. With several
    # (PyYAML's two loaders), the multiple goes on each rival as "Nx slower", so
    # every gap is stated; YAMLRocks is then just the fastest reference.
    multi = len(them) > 1

    fig, ax = plt.subplots(figsize=(8.2, 0.95 * n * len(metrics) / 2 + 1.2))
    span = 0.82
    height = span / n * 0.86
    for i, (_, key) in enumerate(metrics):
        for j, (row, color, is_us) in enumerate(bars):
            y = i - span / 2 + span * (j + 0.5) / n
            ax.barh(y, row[key], height=height, color=color, zorder=3)
            if is_us:
                label = (
                    _fmt(us[key])
                    if multi
                    else f"{_mult(primary[key] / us[key])}x faster · {_fmt(us[key])}"
                )
                lc, weight = CYAN, "bold"
            else:
                label = (
                    f"{_fmt(row[key])} · {_mult(row[key] / us[key])}x slower"
                    if multi
                    else _fmt(row[key])
                )
                lc, weight = MUTED, "normal"
            ax.text(
                row[key] + longest * 0.012,
                y,
                label,
                va="center",
                ha="left",
                color=lc,
                fontsize=8.5 if is_us else 8,
                fontweight=weight,
            )

    ax.set_yticks(range(len(metrics)))
    ax.set_yticklabels([name for name, _ in metrics])
    ax.invert_yaxis()
    # Room to the right for the "Nx faster" label on YAMLRocks' (shorter) bar.
    ax.set_xlim(left=0, right=longest * 1.9)
    # The legend sits below the chart, centered, so a wide multi-library row
    # (PyYAML's two loaders plus YAMLRocks) never runs into the top-left title.
    ax.legend(
        handles=[mpl.patches.Patch(color=CYAN, label="YAMLRocks")]
        + [
            mpl.patches.Patch(
                color=COMPETITOR_COLORS[j % len(COMPETITOR_COLORS)],
                label=_labelled(label, display),
            )
            for j, (label, display) in enumerate(competitors)
        ],
        loc="upper center",
        bbox_to_anchor=(0.5, -0.12),
        ncol=n,
        frameon=False,
        fontsize=9,
        labelcolor=TEXT,
    )
    title_rival = them[0][1] if len(them) == 1 else "PyYAML"
    # No xlabel: it would collide with the legend below, and the bar labels
    # already carry the times, with the multiples telling the direction.
    _style(ax, f"YAMLRocks vs {title_rival}", "")
    # The times are on the bars, so the x-axis ticks and grid are just noise.
    ax.set_xticks([])
    ax.xaxis.grid(visible=False)

    path = OUT / filename
    fig.savefig(path, format="svg", bbox_inches="tight")
    plt.close(fig)
    cleaned = "\n".join(line.rstrip() for line in path.read_text().splitlines())
    path.write_text(cleaned + "\n")


def main() -> None:
    """Render the field charts and the per-page head-to-head charts."""
    OUT.mkdir(parents=True, exist_ok=True)
    rows = compare.measure()
    _chart(rows, "load_us", "Parsing (loads), across libraries", "load.svg")
    _chart(rows, "dump_us", "Serializing (dumps), across libraries", "dump.svg")
    for filename, competitors in _HEAD_TO_HEAD:
        _head_to_head(rows, competitors, filename)
    print(f"wrote charts to {OUT}")
    # Echo the measured numbers so the doc tables and the index leaderboard can be
    # updated from the very same run the charts were drawn from (no drift).
    print(f"\n{'library':<14} {'load':>10} {'dump':>10}")
    for row in rows:
        dump = "-" if row["dump_us"] is None else _fmt(row["dump_us"])
        print(f"{row['label']:<14} {_fmt(row['load_us']):>10} {dump:>10}")


if __name__ == "__main__":
    main()
