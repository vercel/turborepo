from app.client import hello


def test_hello():
    assert hello() is not None
