"""Installation behavior: replace fork binaries without touching player data."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("fork_tools", Path(__file__).resolve().parents[1] / "tools/fork.py")
fork = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(fork)


class ForkInstallTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="horizon-fork-tools-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.home = self.root / "launcher"
        self.home.mkdir()
        self.app = self.root / "app"
        self.app.mkdir()
        self.env = {"HORIZON_LAUNCHER_HOME": str(self.home),
                    "HORIZON_FORK_INSTALL_DIR": str(self.app),
                    "XDG_DATA_HOME": str(self.root / "data"),
                    "CARGO_TARGET_DIR": str(self.root / "target")}
        artifacts = self.root / "target/fast"
        artifacts.mkdir(parents=True)
        for name in ("horizon-client", "horizon-ctl", "horizon-launcher"):
            p = artifacts / name
            p.write_bytes(b"new-" + name.encode())
            p.chmod(0o755)
        self.write_json(self.home / "settings.json", {"selected": "native-existing", "accent": "violet"})
        self.write_json(self.home / "instances.json", {"instances": {
            "horizon": {"version": "native-existing", "played_secs": 123},
            "another": {"version": "keep-this"}}})
        self.write_json(self.home / "accounts.json", {"selected": "account", "accounts": ["unchanged"]})
        self.app.joinpath("config.json").write_text('{"existing_direct_start_config":true}\n')
        self.world = self.root / "data/horizon/saves/My World/level.dat"
        self.world.parent.mkdir(parents=True)
        self.world.write_bytes(b"world data")
        self.skin = self.root / "data/horizon/assets/skins/ab/hash"
        self.skin.parent.mkdir(parents=True)
        self.skin.write_bytes(b"cached skin")
        self.account_bytes = (self.home / "accounts.json").read_bytes()
        self.direct_config = (self.app / "config.json").read_bytes()
        self.resources = self.home / "fork-versions/native-existing/.local/horizon-client/26.3"
        self.resources.mkdir(parents=True)
        self.write_json(self.resources / "horizon-resources.json", {"minecraft": "26.3"})
        for name in ("registries.json", "commands.json"):
            self.resources.joinpath(name).write_text("{}")

    @staticmethod
    def write_json(path, value):
        path.write_text(json.dumps(value))

    def assert_player_data_unchanged(self):
        self.assertEqual((self.home / "accounts.json").read_bytes(), self.account_bytes)
        self.assertEqual((self.app / "config.json").read_bytes(), self.direct_config)
        self.assertEqual(self.world.read_bytes(), b"world data")
        self.assertEqual(self.skin.read_bytes(), b"cached skin")

    def test_install_both_and_repeat_preserves_player_data_and_settings(self):
        for _ in range(2):
            fork.install("all", "fast", self.env)
            self.assert_player_data_unchanged()
        game = self.home / "fork-versions/fork-local"
        self.assertEqual((game / "horizon-client").read_bytes(), b"new-horizon-client")
        self.assertTrue((game / "horizon-client").stat().st_mode & 0o111)
        self.assertEqual((self.app / "horizon-launcher").read_bytes(), b"new-horizon-launcher")
        self.assertTrue((game / ".installed").exists())
        settings = json.loads((self.home / "settings.json").read_text())
        self.assertEqual(settings, {"selected": "fork-local", "accent": "violet"})
        instances = json.loads((self.home / "instances.json").read_text())["instances"]
        self.assertEqual(instances["horizon"], {"version": "fork-local", "played_secs": 123})
        self.assertEqual(instances["another"], {"version": "keep-this"})
        self.assertTrue((self.resources / "commands.json").exists())
        self.assertIn(str(self.app / "horizon-launcher"),
                      (self.root / "data/applications/horizon-launcher.desktop").read_text())

    def test_launcher_only_does_not_change_selected_game_or_require_resources(self):
        self.resources.joinpath("commands.json").unlink()
        settings = (self.home / "settings.json").read_bytes()
        instances = (self.home / "instances.json").read_bytes()
        fork.install("launcher", "fast", self.env)
        self.assertEqual((self.home / "settings.json").read_bytes(), settings)
        self.assertEqual((self.home / "instances.json").read_bytes(), instances)
        self.assertFalse((self.home / "fork-versions/fork-local").exists())
        self.assert_player_data_unchanged()

    def test_missing_resources_fails_before_replacing_launcher_or_selection(self):
        (self.app / "horizon-launcher").write_bytes(b"old launcher")
        self.resources.joinpath("commands.json").unlink()
        settings = (self.home / "settings.json").read_bytes()
        with self.assertRaisesRegex(RuntimeError, "No prepared Minecraft"):
            fork.install("all", "fast", self.env)
        self.assertEqual((self.app / "horizon-launcher").read_bytes(), b"old launcher")
        self.assertEqual((self.home / "settings.json").read_bytes(), settings)
        self.assertFalse((self.home / "fork-versions/fork-local").exists())
        self.assert_player_data_unchanged()


if __name__ == "__main__":
    unittest.main()
