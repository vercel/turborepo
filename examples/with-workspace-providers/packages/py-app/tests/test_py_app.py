from py_app import headline


def test_headline_title_cases_words():
    assert headline(["mixed", "workspace", "providers"]) == "Mixed Workspace Providers"
