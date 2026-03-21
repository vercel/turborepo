from mathlib import normalize, weighted_sum


def make_report(values: list[float]) -> dict[str, float]:
    weights = normalize(values)
    score = weighted_sum(values, weights)
    return {
        "score": round(score, 3),
        "max": max(values, default=0.0),
        "min": min(values, default=0.0),
    }
