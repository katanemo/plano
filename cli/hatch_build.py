"""Hatch build hook to bundle config files from the repo into the package.

Single source of truth stays in ../config/. This hook copies them into
planoai/data/ so they end up inside both the sdist and wheel.
"""

import os
import shutil

from hatchling.builders.hooks.plugin.interface import BuildHookInterface

FILES = {
    "plano_config_schema.yaml": "plano_config_schema.yaml",
    "envoy.template.yaml": "envoy.template.yaml",
}


class CustomBuildHook(BuildHookInterface):
    def initialize(self, version, build_data):
        root = os.path.dirname(__file__)
        # Repo checkout: ../config/  |  sdist: config/
        candidates = [
            os.path.join(root, "..", "config"),
            os.path.join(root, "config"),
        ]
        dest_dir = os.path.join(root, "planoai", "data")
        os.makedirs(dest_dir, exist_ok=True)

        for src_name, dest_name in FILES.items():
            dest = os.path.join(dest_dir, dest_name)
            copied = False
            for cand in candidates:
                src = os.path.join(cand, src_name)
                if os.path.exists(src):
                    shutil.copy2(src, dest)
                    copied = True
                    break
            if not copied and not os.path.exists(dest):
                raise FileNotFoundError(
                    f"Config file {src_name} not found. "
                    "Build from the repo root or ensure files are present."
                )

        build_data["force_include"][dest_dir] = "planoai/data"
