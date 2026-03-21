from analytics import make_report


def test_make_report_has_basic_stats():
    report = make_report([2.0, 3.0, 5.0])
    assert report["score"] == 3.8
    assert report["max"] == 5.0
    assert report["min"] == 2.0
