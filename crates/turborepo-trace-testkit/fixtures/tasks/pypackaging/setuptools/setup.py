from setuptools import setup

DATA_FILES = [("etc/stpkg", ["conf/default.cfg"])]

setup(
    data_files=DATA_FILES,
    py_modules=["standalone"],
    version=read_version(),
    package_data=extra_package_data(),
)
