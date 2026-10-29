"""The opt-in validated reference profile must not waive the strict output gate."""
from __future__ import annotations
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
import zipfile
from tools import prepare_output

class ReferenceProfileTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.resources, self.source = self.root / "resources", self.root / "source"
        self.resources.mkdir(); self.source.mkdir()
        self.jar = self.root / "client.jar"
        self.index = self.root / "index.json"
        self.version = self.root / "version.json"
        self.lock = self.root / "research-lock.json"
        self.oracle = "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaHeightmapOracle.java"
        source = self.source / self.oracle
        source.parent.mkdir(parents=True)
        source.write_bytes(b"class Oracle {\n}\n")
        policy = "crates/horizon-prepare/src/sounds.rs"
        path = self.source / policy; path.parent.mkdir(parents=True)
        path.write_bytes((prepare_output.ROOT / policy).read_bytes())
        self.files = {"facts.json": b'{"value":1}', "model-states.json": b'{}',
                      "block-tags.json": b'{\n"tags": []\n}\n', "cached-collision.nbt": b"nbt",
                      "core.nbt": b"core"}
        self.optional = {"pack.mcmeta": b'{"language":{}}',
                         "minecraft/lang/zz_zz.json": b'{"name":"Test"}'}
        self.jar_files = {"assets/minecraft/lang/en_us.json": b'{"name":"English"}'}
        with zipfile.ZipFile(self.jar, "w") as jar:
            for name, data in self.jar_files.items(): jar.writestr(name, data)
        objects = {name: {"hash": hashlib.sha1(data).hexdigest(), "size": len(data)}
                   for name, data in self.optional.items()}
        self.index.write_text(json.dumps({"objects": objects}))
        self.version.write_text(json.dumps({"id":"test", "assetIndex":{"sha1":hashlib.sha1(self.index.read_bytes()).hexdigest()},
            "downloads":{"client":{"sha1":hashlib.sha1(self.jar.read_bytes()).hexdigest()}}}))
        lock = {"minecraft":{"version":"test", "manifest_sha1":hashlib.sha1(self.version.read_bytes()).hexdigest(),
                              "client_sha1":hashlib.sha1(self.jar.read_bytes()).hexdigest()}}
        self.lock.write_text(json.dumps(lock))
        (self.source / "research-lock.json").write_bytes(self.lock.read_bytes())
        self.manifest = {"schema":7, "minecraft":"test", "data_version":1, "resource_files":1,
            "client_sha256":hashlib.sha256(self.jar.read_bytes()).hexdigest(), "server_sha256":"a"*64,
            "exporter_sha256":hashlib.sha256(source.read_bytes()).hexdigest(),
            "facts_sha256":hashlib.sha256(self.files["facts.json"]).hexdigest(),
            "model_states_sha256":hashlib.sha256(self.files["model-states.json"]).hexdigest(),
            "block_tags_sha256":hashlib.sha256(self.files["block-tags.json"]).hexdigest(),
            "collision_sha256":hashlib.sha256(self.files["cached-collision.nbt"]).hexdigest(),
            "motion_exporters_sha256":{self.oracle:hashlib.sha256(source.read_bytes()).hexdigest()},
            "motion_files_sha256":{"core.nbt":hashlib.sha256(self.files["core.nbt"]).hexdigest()}}
        schema = {'health_exporters_sha256': ['mods/horizon/src/test/java/dev/horizonlab/parity/VanillaDamageSourceOracle.java', 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaPlayerHealthOracle.java', 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEntityAttributeOracle.java'], 'health_files_sha256': ['damage-types.json', 'damage-tags.json', 'damage-catalog.json', 'player-float-ids.json'], 'motion_exporters_sha256': ['mods/horizon/src/test/java/dev/horizonlab/parity/VanillaMovementRegistryOracle.java', 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEntityAttributeOracle.java', 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaHeightmapOracle.java', 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBlockLandingOracle.java', 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCollisionOracle.java'], 'motion_files_sha256': ['movement-properties.nbt', 'attributes.json', 'landing-definitions.json'], 'collision_exporters_sha256': ['mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCollisionOracle.java'], 'render_exporters_sha256': ['mods/horizon/src/test/java/dev/horizonlab/parity/VanillaFaceVisibilityOracle.java', 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCollisionOracle.java', 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaHeightmapOracle.java'], 'render_files_sha256': ['render-definitions.json', 'occlusion.nbt'], 'model_state_exporters_sha256': ['mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBlockStateBindingOracle.java', 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCuboidModelOracle.java'], 'gameplay_exporters_sha256': ['mods/horizon/src/test/java/dev/horizonlab/parity/VanillaGameplayFactsExporter.java'], 'entity_model_exporters_sha256': ['mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEntityModelExporter.java'], 'creative_exporters_sha256': ['mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCreativeTabsExporter.java', 'mods/horizon/src/test/java/dev/horizonlab/parity/ParityData.java'], 'mob_data_exporters_sha256': ['mods/horizon/src/test/java/dev/horizonlab/parity/VanillaMobDataExporter.java'], 'entity_types_exporters_sha256': ['mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEntityTypesExporter.java'], 'behaviour_exporters_sha256': ['mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBlockBehaviourExporter.java'], 'command_registries_exporters_sha256': ['mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCommandParseOracle.java']}
        self.manifest.pop("motion_files_sha256")
        for group, names in schema.items():
            hashes = {}
            for name in names:
                if group.endswith("_exporters_sha256"):
                    path = self.source / name; path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(b"class Oracle {\n}\n"); data = path.read_bytes()
                else:
                    data = self.files.setdefault(name, b"{}"); self.write(name, data)
                hashes[name] = hashlib.sha256(data).hexdigest()
            self.manifest[group] = hashes
        for name, data in {**self.files, **self.jar_files}.items(): self.write(name, data)
        self.write_manifest()
        self.baseline = prepare_output.digest(self.resources)
        for name, data in self.optional.items(): self.write("assets/" + name, data)
        self.write("asset-index.json", self.index.read_bytes())

    def write(self, name: str, data: bytes) -> None:
        path = self.resources / name; path.parent.mkdir(parents=True, exist_ok=True); path.write_bytes(data)

    def write_manifest(self) -> None:
        self.write("horizon-resources.json", json.dumps(self.manifest).encode())

    def profile(self):
        # Before the implementation exists, positive/negative behavior assertions fail clearly.
        function = getattr(prepare_output, "reference_digest", lambda *args: {})
        return function(self.resources, self.source, self.jar, self.index, self.version, self.lock)

    def test_validated_optional_language_preserves_full_bytes_and_baseline(self) -> None:
        raw = prepare_output.digest(self.resources)
        self.assertNotEqual(raw, self.baseline)
        result = self.profile()
        self.assertEqual(result.get("core_digest"), self.baseline[0])
        self.assertEqual(result.get("raw_digest"), raw[0])
        self.assertEqual(result.get("source_hash_entries_verified"), 23)
        self.assertEqual(prepare_output.digest(self.resources), raw)
        self.assertEqual((self.resources / "assets/minecraft/lang/zz_zz.json").read_bytes(), self.optional["minecraft/lang/zz_zz.json"])

    def test_only_two_verified_crlf_identities_project_to_lf(self) -> None:
        path = self.source / self.oracle; path.write_bytes(path.read_bytes().replace(b"\n", b"\r\n"))
        self.manifest["exporter_sha256"] = hashlib.sha256(path.read_bytes()).hexdigest()
        for group, values in self.manifest.items():
            if group.endswith("_exporters_sha256") and self.oracle in values:
                values[self.oracle] = self.manifest["exporter_sha256"]
        tags = self.resources / "block-tags.json"; tags.write_bytes(tags.read_bytes().replace(b"\n", b"\r\n"))
        self.manifest["block_tags_sha256"] = hashlib.sha256(tags.read_bytes()).hexdigest(); self.write_manifest()
        self.assertEqual(self.profile().get("core_digest"), self.baseline[0])

    def test_invalid_optional_bytes_size_index_jar_or_source_cannot_be_projected(self) -> None:
        for relative in ["assets/minecraft/lang/zz_zz.json", "movement-properties.nbt"]:
            with self.subTest(relative=relative):
                path = self.resources / relative; old = path.read_bytes(); path.write_bytes(old+b"bad")
                try:
                    with self.assertRaises(ValueError): self.profile()
                finally: path.write_bytes(old)
        for path in [self.index, self.jar, self.version, self.source / self.oracle, self.source / "research-lock.json"]:
            with self.subTest(path=path.name):
                old = path.read_bytes(); path.write_bytes(old+b"bad")
                try:
                    with self.assertRaises(ValueError): self.profile()
                finally: path.write_bytes(old)

    def test_partial_claimed_family_and_stale_manifest_fail(self) -> None:
        path = self.resources / "assets/minecraft/lang/zz_zz.json"; old = path.read_bytes(); path.unlink()
        with self.assertRaises(ValueError): self.profile()
        path.write_bytes(old)
        self.manifest["block_tags_sha256"] = "0"*64; self.write_manifest()
        with self.assertRaises(ValueError): self.profile()

    def test_unrelated_extra_and_semantic_core_changes_remain_in_digest(self) -> None:
        self.write("new.nbt", b"unexpected")
        result = self.profile()
        self.assertNotEqual(result.get("core_digest"), self.baseline[0])
        (self.resources / "new.nbt").unlink()
        self.write("facts.json", b'{"value":2}')
        self.manifest["facts_sha256"] = hashlib.sha256(b'{"value":2}').hexdigest(); self.write_manifest()
        self.assertNotEqual(self.profile().get("core_digest"), self.baseline[0])

    def test_changed_jar_path_is_held_even_with_verified_bytes_projection(self) -> None:
        from unittest.mock import patch
        from tools import prepare_output_profile
        real_zip = zipfile.ZipFile
        original = self.jar.read_bytes()
        def replace_at_open(file, *args, **kwargs):
            with real_zip(self.jar, "w") as changed:
                changed.writestr("assets/minecraft/lang/zz_zz.json", b"forged placeholder")
            return real_zip(file, *args, **kwargs)
        try:
            with patch.object(prepare_output_profile.zipfile, "ZipFile", replace_at_open), self.assertRaisesRegex(ValueError, "reference input changed"):
                self.profile()
        finally:
            self.jar.write_bytes(original)

    def test_all_optional_families_restore_verified_jar_placeholders_and_keep_full_assets(self) -> None:
        for name in self.optional:
            (self.resources / ("assets/" + name)).unlink()
        (self.resources / "asset-index.json").unlink()
        additions = {"minecraft/font/include/unifont.json":b'{"providers":[1]}',
                     "minecraft/font/unifont.zip":b"font-bytes",
                     "minecraft/resourcepacks/high_contrast.zip":b"pack-one",
                     "minecraft/resourcepacks/programmer_art.zip":b"pack-two"}
        panorama = [f"minecraft/textures/gui/title/background/panorama_{n}.png" for n in range(6)]
        panorama += ["minecraft/textures/gui/title/background/panorama_overlay.png"]
        additions.update({name:("full-" + name).encode() for name in panorama})
        placeholders = {"assets/" + name:b"placeholder" for name in panorama}
        placeholders["assets/minecraft/font/include/unifont.json"] = b'{"providers":[]}'
        self.jar_files.update(placeholders)
        with zipfile.ZipFile(self.jar, "w") as jar:
            for name, data in self.jar_files.items(): jar.writestr(name,data)
        for name, data in self.jar_files.items(): self.write(name,data)
        self.manifest["client_sha256"] = hashlib.sha256(self.jar.read_bytes()).hexdigest()
        self.write_manifest(); baseline = prepare_output.digest(self.resources)
        self.optional.update(additions)
        objects = {name:{"hash":hashlib.sha1(data).hexdigest(),"size":len(data)} for name,data in self.optional.items()}
        self.index.write_text(json.dumps({"objects":objects}))
        version = json.loads(self.version.read_text()); version["assetIndex"]["sha1"] = hashlib.sha1(self.index.read_bytes()).hexdigest()
        version["downloads"]["client"]["sha1"] = hashlib.sha1(self.jar.read_bytes()).hexdigest()
        self.version.write_text(json.dumps(version))
        lock = json.loads(self.lock.read_text()); lock["minecraft"]["client_sha1"] = version["downloads"]["client"]["sha1"]
        lock["minecraft"]["manifest_sha1"] = hashlib.sha1(self.version.read_bytes()).hexdigest()
        self.lock.write_text(json.dumps(lock)); (self.source/"research-lock.json").write_bytes(self.lock.read_bytes())
        for name,data in self.optional.items(): self.write("assets/"+name,data)
        self.write("asset-index.json",self.index.read_bytes())
        raw = prepare_output.digest(self.resources)
        result = self.profile()
        self.assertEqual(result["core_digest"],baseline[0])
        self.assertEqual(result["optional_files_verified"],13)
        self.assertEqual(prepare_output.digest(self.resources),raw)
        for name in ["minecraft/font/unifont.zip","minecraft/resourcepacks/programmer_art.zip",panorama[0]]:
            with self.subTest(corrupt_optional=name):
                self.write("assets/"+name,b"wrong")
                try:
                    with self.assertRaises(ValueError): self.profile()
                finally: self.write("assets/"+name,self.optional[name])
        (self.resources / ("assets/"+panorama[1])).unlink()
        with self.assertRaises(ValueError): self.profile()

    def test_missing_integrity_member_or_unknown_nested_manifest_data_fails(self) -> None:
        original = dict(self.manifest["health_exporters_sha256"])
        self.manifest["health_exporters_sha256"].pop(next(iter(original))); self.write_manifest()
        with self.assertRaises(ValueError): self.profile()
        self.manifest["health_exporters_sha256"] = original
        self.write_manifest()
        path = self.resources / "asset-index.json"; path.unlink()
        with self.assertRaises(ValueError): self.profile()

    def test_reference_reparse_metadata_is_rejected_before_read(self) -> None:
        from unittest.mock import patch
        from types import SimpleNamespace
        actual = Path.lstat
        wanted = self.source / self.oracle
        def metadata(path, *args, **kwargs):
            if path == wanted:
                return SimpleNamespace(st_mode=0o100644, st_file_attributes=0x400)
            return actual(path,*args,**kwargs)
        with patch.object(Path,"lstat",metadata), self.assertRaises(ValueError): self.profile()

    def test_pinned_index_size_mismatch_and_unsafe_key_fail(self) -> None:
        def repin_index() -> None:
            version = json.loads(self.version.read_text())
            version["assetIndex"]["sha1"] = hashlib.sha1(self.index.read_bytes()).hexdigest()
            self.version.write_text(json.dumps(version))
            lock = json.loads(self.lock.read_text()); lock["minecraft"]["manifest_sha1"] = hashlib.sha1(self.version.read_bytes()).hexdigest()
            self.lock.write_text(json.dumps(lock)); (self.source/"research-lock.json").write_bytes(self.lock.read_bytes())
        index = json.loads(self.index.read_text()); index["objects"]["minecraft/lang/zz_zz.json"]["size"] += 1
        self.index.write_text(json.dumps(index)); repin_index()
        self.write("asset-index.json", self.index.read_bytes())
        with self.assertRaisesRegex(ValueError,"optional asset"): self.profile()
        index["objects"]["minecraft/lang/zz_zz.json"]["size"] -= 1
        index["objects"]["minecraft/lang/zz_zz.json"]["unvalidated"] = True
        self.index.write_text(json.dumps(index)); repin_index()
        self.write("asset-index.json", self.index.read_bytes())
        with self.assertRaisesRegex(ValueError,"asset record"): self.profile()
        index["objects"]["minecraft/lang/zz_zz.json"].pop("unvalidated")
        index["objects"]["../outside"] = {"hash":"0"*40,"size":0}
        self.index.write_text(json.dumps(index)); repin_index()
        self.write("asset-index.json",self.index.read_bytes())
        with self.assertRaisesRegex(ValueError,"unsafe"): self.profile()

    def test_windows_superscript_device_aliases_are_rejected(self) -> None:
        from tools import prepare_output_profile
        for name in ["COM\u00b9.java", "LPT\u00b2", "COM\u00b3.txt"]:
            with self.subTest(name=ascii(name)), self.assertRaises(ValueError):
                prepare_output_profile.relative(name)

    def test_nonreserved_unicode_device_lookalike_is_not_rejected(self) -> None:
        from tools import prepare_output_profile
        self.assertEqual(prepare_output_profile.relative("COM\u00c3.java"), "COM\u00c3.java")

    def test_changed_validated_resource_or_reference_at_raw_digest_boundary_is_held(self) -> None:
        from unittest.mock import patch
        targets = [self.resources / "assets/minecraft/lang/zz_zz.json",
                   self.resources / "block-tags.json", self.resources / "horizon-resources.json",
                   self.source / self.oracle, self.index, self.version, self.lock, self.jar]
        real_digest = prepare_output.digest
        for path in targets:
            with self.subTest(changed=path.name):
                original = path.read_bytes()
                def mutate_then_digest(root):
                    path.write_bytes(original + b"changed-after-validation")
                    return real_digest(root)
                try:
                    with patch.object(prepare_output, "digest", mutate_then_digest), self.assertRaises(ValueError):
                        self.profile()
                finally:
                    path.write_bytes(original)

    def test_real_scanner_io_error_cannot_become_partial_success(self) -> None:
        from unittest.mock import patch
        from tools import prepare_output_profile
        with patch.object(prepare_output_profile.os, "scandir", side_effect=PermissionError("denied")):
            with self.assertRaises(PermissionError): self.profile()

    def test_explicit_cli_profile_checks_baseline_and_refuses_cached_or_missing_references(self) -> None:
        from unittest.mock import patch
        versions = self.source / prepare_output.VERSIONS
        versions.parent.mkdir(parents=True, exist_ok=True)
        versions.write_text("1 " + self.baseline[0] + "\n")
        library = self.source / prepare_output.LIB
        library.write_text("pub const OUTPUT_VERSION: u32 = 1;\n")
        args = ["prepare_output.py", "check", str(self.resources), "--validated-reference",
                "--reference-source", str(self.source), "--reference-client", str(self.jar),
                "--reference-index", str(self.index), "--reference-version", str(self.version)]
        with patch.object(prepare_output, "ROOT", self.source), patch("sys.argv", args):
            try:
                self.assertEqual(prepare_output.main(), 0)
            except SystemExit as error:
                self.fail(f"explicit valid profile did not run: {error}")
        with patch.object(prepare_output, "ROOT", self.source), patch("sys.argv", [*args, "--cache"]):
            with self.assertRaises(SystemExit) as failure: prepare_output.main()
            self.assertNotEqual(failure.exception.code, 0)
        with patch("sys.argv", args[:4]):
            with self.assertRaises(SystemExit) as failure: prepare_output.main()
            self.assertNotEqual(failure.exception.code, 0)

    def test_manifest_traversal_and_unknown_container_are_rejected(self) -> None:
        self.manifest["motion_exporters_sha256"] = {"../escape.java":"0"*64}; self.write_manifest()
        with self.assertRaises(ValueError): self.profile()
        self.manifest["motion_exporters_sha256"] = {self.oracle:hashlib.sha256((self.source/self.oracle).read_bytes()).hexdigest()}
        self.manifest["unknown_hidden_container"] = {}; self.write_manifest()
        with self.assertRaises(ValueError): self.profile()

if __name__ == "__main__": unittest.main()
