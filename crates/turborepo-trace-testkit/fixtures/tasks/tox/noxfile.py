import nox

nox.options.sessions = ["tests", "lint"]


@nox.session(python=["3.11", "3.12"])
def tests(session):
    session.install("-r", "requirements-test.txt")
    session.install(".")
    session.run(
        "pytest",
        "tests",  # unit tests
        *session.posargs,
        env={"PYTHONHASHSEED": "0"},
    )
    session.notify("lint")


@nox.session(name="lint")
def lint_session(session):
    session.install("ruff")
    session.chdir("src")
    session.run("ruff", "check", ".")


@nox.session
def docs(session):
    _build_docs(session)
    session.run("sphinx-build", "docs", f"{OUT}/html")
