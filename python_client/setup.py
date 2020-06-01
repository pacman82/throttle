from setuptools import setup

with open("../Readme.md", "r") as file:
    long_description = file.read()

setup(
    name="throttle_client",
    version="0.3.8",
    author="Markus Klein",
    description="Client for Throttle. Throttle is a http semaphore service, providing"
    "semaphores for distributed systems.",
    long_description=long_description,
    long_description_content_type="text/markdown",
    url="https://github.com/pacman82/throttle.git",
    packages=["throttle_client"],
    classifiers=[
        "Programming Language :: Python :: 3",
        "License :: OSI Approved :: MIT License",
        "Operating System :: OS Independent",
    ],
    install_requires=["requests", "tenacity"],
    python_requires=">=3.6",
)
