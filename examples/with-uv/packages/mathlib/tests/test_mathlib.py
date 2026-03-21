from mathlib import normalize, weighted_sum


def test_normalize():
    assert normalize([2.0, 2.0]) == [0.5, 0.5]


def test_weighted_sum():
    assert weighted_sum([3.0, 5.0], [0.25, 0.75]) == 4.5
