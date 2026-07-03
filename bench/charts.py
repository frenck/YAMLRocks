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
GREY = "#5b626d"  # every other library

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
    labels = [row["label"] for row in rows]
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


def main() -> None:
    """Render the load and dump charts into the docs assets."""
    OUT.mkdir(parents=True, exist_ok=True)
    rows = compare.measure()
    _chart(rows, "load_us", "Parsing (loads), across libraries", "load.svg")
    _chart(rows, "dump_us", "Serializing (dumps), across libraries", "dump.svg")
    print(f"wrote charts to {OUT}")


if __name__ == "__main__":
    main()
