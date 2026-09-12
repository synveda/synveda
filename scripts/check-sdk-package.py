"""ADPT-4: build with Hatchling, then test the installed wheel in a clean venv."""

import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import venv
import zipfile
from importlib.metadata import version
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "sdks/python"
LOCK = SOURCE / "requirements-dev.lock"


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def run(args, cwd):
    subprocess.run(
        args,
        cwd=cwd,
        check=True,
        timeout=120,
        env={
            **os.environ,
            "PIP_NO_INDEX": "1",
            "PIP_DISABLE_PIP_VERSION_CHECK": "1",
            "SOURCE_DATE_EPOCH": "1704067200",
        },
    )


def build(stage):
    shutil.copytree(
        SOURCE,
        stage,
        ignore=shutil.ignore_patterns(".venv", "__pycache__", "*.pyc", "dist"),
    )
    output = stage / "dist"
    output.mkdir()
    run(
        [
            sys.executable,
            "-I",
            "-c",
            "from hatchling.build import build_sdist; build_sdist('dist')",
        ],
        stage,
    )
    archives = list(output.glob("*.tar.gz"))
    assert len(archives) == 1
    extracted = stage / "extracted"
    extracted.mkdir()
    with tarfile.open(archives[0]) as archive:
        archive.extractall(extracted, filter="data")
    projects = list(extracted.iterdir())
    assert len(projects) == 1 and projects[0].is_dir()
    run(
        [
            sys.executable,
            "-I",
            "-c",
            "import sys; from hatchling.build import build_wheel; build_wheel(sys.argv[1])",
            str(output),
        ],
        projects[0],
    )
    wheels = list(output.glob("*.whl"))
    assert len(wheels) == 1
    return archives[0], wheels[0]


def main():
    wheelhouse_value = os.environ.get("SYNVEDA_SDK_WHEELHOUSE")
    if not wheelhouse_value:
        raise SystemExit(
            "Set SYNVEDA_SDK_WHEELHOUSE to wheels downloaded from requirements-dev.lock with --require-hashes"
        )
    wheelhouse = Path(wheelhouse_value).resolve(strict=True)
    assert wheelhouse.is_dir()
    # The offline installer may choose only bytes admitted by the repository lock.
    allowed = set(re.findall(r"sha256:([0-9a-f]{64})", LOCK.read_text()))
    supplied = list(wheelhouse.glob("*.whl"))
    assert supplied and all(
        digest(wheel) in allowed for wheel in supplied
    ), "wheelhouse contains unlocked bytes"
    metadata = tomllib.loads((SOURCE / "pyproject.toml").read_text())
    assert metadata["build-system"]["requires"] == [
        "hatchling==" + version("hatchling")
    ]

    with tempfile.TemporaryDirectory(prefix="synveda-sdk-wheel-") as temporary:
        scratch = Path(temporary)
        first = build(scratch / "first")
        second = build(scratch / "second")
        assert [digest(path) for path in first] == [
            digest(path) for path in second
        ], "two clean Python builds must agree"
        wheel = first[1]
        with zipfile.ZipFile(wheel) as archive:
            for name in (
                "__init__.py",
                "client.py",
                "models.py",
                "operations.py",
                "contract.json",
                "py.typed",
            ):
                assert (
                    archive.read("synveda/" + name)
                    == (SOURCE / "synveda" / name).read_bytes()
                )
            assert not any(
                "__pycache__" in name or name.startswith("tests/")
                for name in archive.namelist()
            )
        consumer = scratch / "consumer"
        venv.EnvBuilder(with_pip=True).create(consumer)
        python = consumer / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
        run(
            [
                str(python),
                "-I",
                "-m",
                "pip",
                "install",
                "--no-index",
                "--no-cache-dir",
                "--only-binary=:all:",
                "--find-links",
                str(wheelhouse),
                str(wheel),
            ],
            scratch,
        )
        run([str(python), "-I", "-m", "pip", "check"], scratch)
        run(
            [
                str(python),
                "-I",
                "-c",
                """
import importlib.util, json, sys
from importlib.resources import files
from pathlib import Path
import synveda
from synveda import models, operations
assert Path(synveda.__file__).is_relative_to(Path(sys.prefix))
assert importlib.util.find_spec('hatchling') is None
assert importlib.util.find_spec('datamodel_code_generator') is None
contract = json.loads(files('synveda').joinpath('contract.json').read_text())
assert contract['openapi_sha256'] == sys.argv[1]
assert len(contract['operations']) == 15
assert files('synveda').joinpath('py.typed').is_file()
assert callable(operations.open_session) and models.OpenSessionBody
""",
                digest(ROOT / "docs/api/openapi.json"),
            ],
            scratch,
        )
        # -I ignores PYTHONPATH/user site; the test directory cannot shadow synveda.
        run(
            [
                str(python),
                "-I",
                "-m",
                "unittest",
                "discover",
                "-s",
                str(SOURCE / "tests"),
                "-v",
            ],
            scratch,
        )
        print(
            json.dumps(
                {
                    "package": metadata["project"]["name"],
                    "python": sys.version.split()[0],
                    "sdist_sha256": digest(first[0]),
                    "wheel_sha256": digest(wheel),
                    "clean_builds_identical": True,
                    "installed_resources_and_types": True,
                }
            )
        )


if __name__ == "__main__":
    main()
