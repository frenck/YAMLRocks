"""Compare live cursor searches with exporting and walking a bulk snapshot."""

import statistics
import time

import yamlrocks


def cursor_walk(node, values):
    """Read the same presentation fields for each mapping value."""
    for key, value in values.items():
        child = node[key]
        _ = child.style, child.tag, child.comment, child.comment_before
        if isinstance(value, dict):
            cursor_walk(child, value)


def snapshot_walk(tree):
    """Visit an owned snapshot without searching mapping keys."""
    _ = tree.style, tree.tag, tree.comment, tree.comment_before
    if tree.kind == "mapping":
        for key, value in tree.value:
            snapshot_walk(key)
            snapshot_walk(value)
    elif tree.kind == "sequence":
        for child in tree.value:
            snapshot_walk(child)


def median_ms(function):
    """Measure three complete calls, including snapshot creation and cleanup."""
    samples = []
    for _ in range(3):
        start = time.perf_counter()
        function()
        samples.append((time.perf_counter() - start) * 1000)
    return statistics.median(samples)


if __name__ == "__main__":
    print("entries,cursor_ms,snapshot_export_and_walk_ms", flush=True)
    for count in (1000, 2000, 4000, 8000):
        values = {f"key_{index:05}": {"value": "text"} for index in range(count)}
        doc = yamlrocks.loads(yamlrocks.dumps(values), option=yamlrocks.OPT_ROUND_TRIP)
        cursor = median_ms(lambda: cursor_walk(doc.node, values))
        snapshot = median_ms(lambda: snapshot_walk(doc.to_tree()))
        print(f"{count},{cursor:.3f},{snapshot:.3f}", flush=True)
