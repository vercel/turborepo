from py_util import title_case_words


def headline(words: list[str]) -> str:
    return " ".join(title_case_words(words))
