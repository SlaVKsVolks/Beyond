"""Opt-in, integrity-checked projection to the historical JAR-only output profile.

No resources are modified. The original strict digest/check is a separate identity.
This is developer tooling, never a dependency of the game or native preparer.
"""
from __future__ import annotations

import hashlib
from io import BytesIO
import json
import os
from pathlib import Path
import re
import stat
import zipfile
from typing import NoReturn

from . import prepare_output as output

ORACLE = "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaHeightmapOracle.java"
POLICY = "crates/horizon-prepare/src/sounds.rs"
SCALAR_FILES = {"facts_sha256": "facts.json", "model_states_sha256": "model-states.json",
                "block_tags_sha256": "block-tags.json", "collision_sha256": "cached-collision.nbt"}
# Manifest schema 7, from horizon-prepare/src/lib.rs. Missing groups/members fail,
# rather than letting the legacy canonicalizer hide missing integrity metadata.
GROUPS = {'health_exporters_sha256': ['mods/horizon/src/test/java/dev/horizonlab/parity/VanillaDamageSourceOracle.java',
                             'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaPlayerHealthOracle.java',
                             'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEntityAttributeOracle.java'],
 'health_files_sha256': ['damage-types.json',
                         'damage-tags.json',
                         'damage-catalog.json',
                         'player-float-ids.json'],
 'motion_exporters_sha256': ['mods/horizon/src/test/java/dev/horizonlab/parity/VanillaMovementRegistryOracle.java',
                             'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEntityAttributeOracle.java',
                             'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaHeightmapOracle.java',
                             'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBlockLandingOracle.java',
                             'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCollisionOracle.java'],
 'motion_files_sha256': ['movement-properties.nbt', 'attributes.json', 'landing-definitions.json'],
 'collision_exporters_sha256': ['mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCollisionOracle.java'],
 'render_exporters_sha256': ['mods/horizon/src/test/java/dev/horizonlab/parity/VanillaFaceVisibilityOracle.java',
                             'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCollisionOracle.java',
                             'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaHeightmapOracle.java'],
 'render_files_sha256': ['render-definitions.json', 'occlusion.nbt'],
 'model_state_exporters_sha256': ['mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBlockStateBindingOracle.java',
                                  'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCuboidModelOracle.java'],
 'gameplay_exporters_sha256': ['mods/horizon/src/test/java/dev/horizonlab/parity/VanillaGameplayFactsExporter.java'],
 'entity_model_exporters_sha256': ['mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEntityModelExporter.java'],
 'creative_exporters_sha256': ['mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCreativeTabsExporter.java',
                               'mods/horizon/src/test/java/dev/horizonlab/parity/ParityData.java'],
 'mob_data_exporters_sha256': ['mods/horizon/src/test/java/dev/horizonlab/parity/VanillaMobDataExporter.java'],
 'entity_types_exporters_sha256': ['mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEntityTypesExporter.java'],
 'behaviour_exporters_sha256': ['mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBlockBehaviourExporter.java'],
 'command_registries_exporters_sha256': ['mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCommandParseOracle.java']}

def plain(path: Path) -> Path:
    """Reject traversal/reparse aliases before reading any explicit reference."""
    if ".." in path.parts:
        raise ValueError("reference path contains traversal")
    path = path.absolute()
    for component in [*reversed(path.parents), path]:
        info = component.lstat()
        if stat.S_ISLNK(info.st_mode) or getattr(info, "st_file_attributes", 0) & 0x400:
            raise ValueError(f"reference reparse boundary: {component.name}")
    return path


def relative(name: str) -> str:
    if not isinstance(name, str):
        raise ValueError("non-string manifest path")
    parts = name.replace("\\", "/").split("/")
    reserved = {"CON", "PRN", "AUX", "NUL", "CLOCK$", "CONIN$", "CONOUT$"}
    reserved |= {prefix + digit for prefix in ("COM", "LPT") for digit in "123456789\u00b9\u00b2\u00b3"}
    if any(not part or part in {".", ".."} or part.endswith((".", " "))
           or part.split(".", 1)[0].upper() in reserved
           or any(ord(char) < 32 or char in '<>:"|?*' for char in part) for part in parts):
        raise ValueError("unsafe manifest/index path")
    return "/".join(parts)


def read(root: Path, name: str) -> bytes:
    return plain(root / relative(name)).read_bytes()


def object_pairs(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON member")
        result[key] = value
    return result


def obj(value: object) -> dict[str, object]:
    if not isinstance(value, dict) or any(not isinstance(key, str) for key in value):
        raise ValueError("expected JSON object")
    return value


def document(data: bytes) -> dict[str, object]:
    return obj(json.loads(data, object_pairs_hook=object_pairs))


def sha(data: bytes, kind: str = "sha256") -> str:
    return hashlib.new(kind, data).hexdigest()


def require_hash(data: bytes, expected: object, kind: str = "sha256") -> None:
    length = 40 if kind == "sha1" else 64
    if (not isinstance(expected, str) or re.fullmatch(f"[0-9a-f]{{{length}}}", expected) is None
            or sha(data, kind) != expected):
        raise ValueError(f"{kind} reference/content mismatch")


def walk_error(error: OSError) -> NoReturn:
    raise error


def files(root: Path) -> list[str]:
    result = []
    for parent, dirs, names in os.walk(plain(root), followlinks=False, onerror=walk_error):
        for name in dirs + names:
            plain(Path(parent) / name)
        for name in names:
            result.append(relative((Path(parent) / name).relative_to(root).as_posix()))
    return sorted(result)


def resource_catalog(root: Path) -> dict[str, tuple[str, str | None]]:
    """Hash one file at a time; never retain the resource corpus in memory."""
    result = {}
    for name in files(root):
        data = read(root, name)
        result[name] = (sha(data), None if output.skipped(name) else sha(output.canonical(name, data)))
    return result


def catalog_digest(catalog: dict[str, tuple[str, str | None]]) -> tuple[str, int]:
    total, count = hashlib.sha256(), 0
    for name, (_, canonical) in sorted(catalog.items()):
        if canonical is not None:
            total.update(f"{name}\0{canonical}\n".encode())
            count += 1
    return total.hexdigest(), count


def policy(source: Path) -> tuple[dict[str, set[str]], dict[str, str]]:
    """Read the native importer's exact required names/prefixes, not this run's files."""
    data = read(source, POLICY).replace(b"\r\n", b"\n")
    current = read(output.ROOT, POLICY).replace(b"\r\n", b"\n")
    if data != current:
        raise ValueError("native optional import policy/source mismatch")
    text = data.decode("utf8")
    required: dict[str, set[str]] = {}
    for family, constant in [("panorama", "PANORAMA"), ("font", "UNIFONT"),
                             ("pack", "BUILT_IN_PACKS"), ("language", "LANGUAGES")]:
        match = re.search(r"const " + constant + r":.*?=\s*\[(.*?)\];", text, re.S)
        if match is None:
            raise ValueError("unrecognized native required-asset declaration")
        names = re.findall(r'"([^"\\]+)"', match[1])
        if not names:
            raise ValueError("empty native required-asset declaration")
        required[family] = {relative(name) for name in names}
    prefixes = {}
    for family, function in [("font", "font"), ("pack", "built_in_pack"), ("language", "language")]:
        match = re.search(r"fn is_" + function + r"\([^{}]*\{([^{}]*)\}", text)
        prefix = re.search(r'name.starts_with\("([^"\\]+)"\)', match[1]) if match else None
        if prefix is None:
            raise ValueError("unrecognized native optional-asset predicate")
        prefixes[family] = prefix[1]
    return required, prefixes


def validated_digest(resources: Path, source_root: Path, client_jar: Path,
                     asset_index: Path, version_manifest: Path, lock_path: Path) -> dict[str, object]:
    """Verify references/payloads before deriving, never replacing, the core identity."""
    resources, source_root = plain(resources), plain(source_root)
    catalog = resource_catalog(resources)
    reference_paths = {client_jar, asset_index, version_manifest, lock_path,
                       source_root / "research-lock.json", source_root / ORACLE,
                       source_root / POLICY, output.ROOT / POLICY,
                       Path(__file__), Path(output.__file__)}
    reference_paths.update(source_root / name for group, names in GROUPS.items()
                           if group.endswith("_exporters_sha256") for name in names)
    reference_hashes = {path: sha(plain(path).read_bytes()) for path in reference_paths}

    def resource_read(name: str) -> bytes:
        data = read(resources, name)
        if name not in catalog or sha(data) != catalog[name][0]:
            raise ValueError("resource input changed after initial snapshot")
        return data

    jar_data, index_data = plain(client_jar).read_bytes(), plain(asset_index).read_bytes()
    version_data, lock_data = plain(version_manifest).read_bytes(), plain(lock_path).read_bytes()
    lock = obj(document(lock_data).get("minecraft"))
    if document(read(source_root, "research-lock.json")) != document(lock_data):
        raise ValueError("selected source lock differs from trusted lock")
    accepted = lock.get("accepted_manifest_sha1", [])
    if not isinstance(accepted, list) or any(not isinstance(item, str) for item in accepted):
        raise ValueError("invalid accepted metadata identities")
    if sha(version_data, "sha1") not in [lock.get("manifest_sha1"), *accepted]:
        raise ValueError("unpinned version metadata")
    version = document(version_data)
    if version.get("id") != lock.get("version"):
        raise ValueError("version metadata identity mismatch")
    downloads = obj(version.get("downloads"))
    if obj(downloads.get("client")).get("sha1") != lock.get("client_sha1"):
        raise ValueError("version/client lock mismatch")
    require_hash(jar_data, lock.get("client_sha1"), "sha1")
    require_hash(index_data, obj(version.get("assetIndex")).get("sha1"), "sha1")
    manifest = document(resource_read(output.MANIFEST))
    if manifest.get("schema") != 7 or manifest.get("minecraft") != lock.get("version"):
        raise ValueError("unsupported preparation schema/version")
    require_hash(jar_data, manifest.get("client_sha256"))
    source_count = resource_count = 0
    for group, members in GROUPS.items():
        values = obj(manifest.get(group))
        normalized = {relative(name): value for name, value in values.items()}
        if len(normalized) != len(values) or set(normalized) != set(members):
            raise ValueError(f"incomplete or unexpected manifest integrity members: {group}")
        source_group = group.endswith("_exporters_sha256")
        for name, expected in normalized.items():
            require_hash(read(source_root, name) if source_group else resource_read(name), expected)
            source_count += int(source_group)
            resource_count += int(not source_group)
    for key, value in manifest.items():
        if isinstance(value, (dict, list)) and key not in GROUPS:
            raise ValueError("unknown manifest container cannot be silently ignored")
    for key, name in SCALAR_FILES.items():
        require_hash(resource_read(name), manifest.get(key))
        resource_count += 1
    oracle = read(source_root, ORACLE)
    require_hash(oracle, manifest.get("exporter_sha256"))
    source_count += 1
    required, prefixes = policy(source_root)

    def family(name: str) -> str | None:
        if name in required["panorama"]:
            return "panorama"
        for label, prefix in prefixes.items():
            if name.startswith(prefix) or name in required[label]:
                return label
        return None

    objects = obj(document(index_data).get("objects"))
    selections: dict[str, tuple[str, dict[str, object]]] = {}
    for key, value in objects.items():
        if relative(key) != key:
            raise ValueError("asset index path is not canonical POSIX")
        record = obj(value)
        if (set(record) != {"hash", "size"} or type(record["size"]) is not int
                or record["size"] < 0 or not isinstance(record["hash"], str)
                or re.fullmatch(r"[0-9a-f]{40}", record["hash"]) is None):
            raise ValueError("invalid asset record")
        selected = family(key)
        if selected:
            selections[key] = (selected, record)
    paths = list(catalog)
    imported: set[str] = set()
    baselines: dict[str, bytes | None] = {}
    with zipfile.ZipFile(BytesIO(jar_data)) as jar:
        names = jar.namelist()
        if len(names) != len(set(names)):
            raise ValueError("duplicate reference JAR members")
        jar_names = set(names)
        for key, (label, expected) in selections.items():
            name = "assets/" + key
            baseline = jar.read(name) if name in jar_names else None
            if name not in paths:
                continue
            data = resource_read(name)
            size = expected.get("size")
            if type(size) is not int or size < 0:
                raise ValueError("invalid indexed asset size")
            if size == len(data) and sha(data, "sha1") == expected.get("hash"):
                imported.add(key)
                baselines[name] = baseline
            elif baseline is None or data != baseline:
                raise ValueError(f"optional asset fails pinned index and JAR reference: {key}")
        claimed = {selections[key][0] for key in imported}
        for label in claimed:
            expected_names = {key for key, (kind, _) in selections.items() if kind == label}
            if not required[label] <= expected_names or not expected_names <= imported:
                raise ValueError(f"partial claimed optional-asset family: {label}")
    index_present = "asset-index.json" in paths
    if imported and not index_present:
        raise ValueError("imported optional assets have no matching resource index")
    if index_present and resource_read("asset-index.json") != index_data:
        raise ValueError("prepared asset index differs from pinned reference")
    raw_digest, raw_files = output.digest(resources)
    if (raw_digest, raw_files) != catalog_digest(catalog):
        raise ValueError("resource bytes changed between validation and raw digest")
    projected = dict(manifest)
    # These two scalar identities encode proven platform text line endings, not
    # semantic output differences. Their ORIGINAL raw bytes were verified above.
    projected["exporter_sha256"] = sha(oracle.replace(b"\r\n", b"\n"))
    projected["block_tags_sha256"] = sha(resource_read("block-tags.json").replace(b"\r\n", b"\n"))
    total, count = hashlib.sha256(), 0
    for name in paths:
        if output.skipped(name) or name == "asset-index.json" and index_present:
            continue
        if name in baselines:
            data = baselines[name]
            if data is None:
                continue
        elif name == output.MANIFEST:
            data = json.dumps(projected).encode()
        else:
            data = resource_read(name)
        value = sha(output.canonical(name, data))
        total.update(f"{name}\0{value}\n".encode())
        count += 1
    if resource_catalog(resources) != catalog:
        raise ValueError("resource inputs changed during profile validation")
    if any(sha(plain(path).read_bytes()) != expected for path, expected in reference_hashes.items()):
        raise ValueError("reference input changed during profile validation")
    return {"raw_digest": raw_digest, "raw_files": raw_files,
            "core_digest": total.hexdigest(), "core_files": count,
            "optional_files_verified": len(imported), "source_hash_entries_verified": source_count,
            "resource_hash_entries_verified": resource_count,
            "references": {"lock_sha256": sha(lock_data), "client_sha256": sha(jar_data),
                           "index_sha256": sha(index_data), "version_sha256": sha(version_data),
                           "native_policy_sha256": sha(read(source_root, POLICY)),
                           "resource_manifest_sha256": catalog[output.MANIFEST][0],
                           "profile_tool_sha256": reference_hashes[Path(__file__)],
                           "strict_tool_sha256": reference_hashes[Path(output.__file__)]},
            "resources_modified": False, "profile_cache": "disabled",
            "complete_resource_hash_readback": True, "reference_hash_readback": True}
