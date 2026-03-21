def normalize(values: list[float]) -> list[float]:
    total = sum(values)
    if total == 0:
        return [0.0 for _ in values]
    return [value / total for value in values]


def weighted_sum(values: list[float], weights: list[float]) -> float:
    return sum(value * weight for value, weight in zip(values, weights, strict=False))
