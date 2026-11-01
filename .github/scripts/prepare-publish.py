"""Validate release refs before running expensive checks or publishing images."""

import os
import re
import subprocess


def git(*args):
    return subprocess.check_output(["git", *args], text=True).strip()


def release_version(tag):
    number = r"(?:0|[1-9][0-9]*)"
    identifier = r"(?:0|[1-9][0-9]*|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*)"
    match = re.fullmatch(
        rf"v({number}\.{number}\.{number})(?:-({identifier}(?:\.{identifier})*))?",
        tag,
    )
    if not match or len(tag[1:]) > 128:
        raise ValueError("Expected vMAJOR.MINOR.PATCH[-PRERELEASE], without build metadata")
    return tag[1:], match[2] is not None


def main():
    ref = os.environ["GITHUB_REF"]
    commit = git("rev-parse", "HEAD")
    version, prerelease = "", False
    if ref.startswith("refs/tags/"):
        version, prerelease = release_version(ref.removeprefix("refs/tags/"))
        subprocess.run(
            ["git", "merge-base", "--is-ancestor", commit, "origin/master"],
            check=True,
        )
    elif ref != "refs/heads/master":
        raise ValueError("Manual publishing requires master or a valid version tag")
    with open(os.environ["GITHUB_OUTPUT"], "a") as output:
        output.write(f"image=ghcr.io/{os.environ['GITHUB_REPOSITORY'].lower()}\n")
        output.write(f"commit={commit}\nversion={version}\n")
        output.write(f"prerelease={str(prerelease).lower()}\n")


if __name__ == "__main__":
    main()
