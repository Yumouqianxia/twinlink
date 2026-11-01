"""Install the pinned upstream EasyTier executable; no TUN driver is installed."""
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import shutil
import urllib.request
import zipfile

VERSION = "v2.6.4"
ROOT = Path(__file__).resolve().parents[1]


def request(url):
    headers = {"User-Agent": "TwinLink-setup/0.1"}
    # Hosted runners share anonymous GitHub API quotas. Use the ephemeral
    # read-only workflow token for API metadata, never for download/CDN URLs.
    token = os.environ.get("GITHUB_TOKEN")
    if token and url.startswith("https://api.github.com/"):
        headers["Authorization"] = "Bearer " + token
    return urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=120)


def main():
    operating_system = {"Windows": "windows", "Darwin": "macos"}.get(platform.system())
    architecture = {"AMD64": "x86_64", "x86_64": "x86_64", "arm64": "aarch64", "aarch64": "aarch64"}.get(platform.machine())
    if not operating_system or not architecture:
        raise SystemExit("This setup supports Windows and macOS x64/arm64 only.")
    if operating_system == "windows" and architecture == "aarch64":
        architecture = "arm64"
    name = f"easytier-{operating_system}-{architecture}-{VERSION}.zip"
    with request(f"https://api.github.com/repos/EasyTier/EasyTier/releases/tags/{VERSION}") as response:
        release = json.load(response)
    asset = next(item for item in release["assets"] if item["name"] == name)
    with request(asset["browser_download_url"]) as response:
        archive_bytes = response.read()
    digest = hashlib.sha256(archive_bytes).hexdigest()
    expected = asset.get("digest")
    if name == "easytier-windows-x86_64-v2.6.4.zip":
        expected = "sha256:27af91e270e554709b048bd32327fefd2dfce5062ae1e8701af7550c6f525f84"
    if not expected or expected != f"sha256:{digest}":
        raise SystemExit("Upstream checksum is absent or does not match; executable was not installed.")
    destination = ROOT / "src-tauri" / "engine"
    destination.mkdir(parents=True, exist_ok=True)
    executable = "easytier-core.exe" if operating_system == "windows" else "easytier-core"
    required = [executable, "Packet.dll"] if operating_system == "windows" else [executable]
    with zipfile.ZipFile(io.BytesIO(archive_bytes)) as archive:
        for filename in required:
            member = next(info for info in archive.infolist() if Path(info.filename).name.lower() == filename.lower())
            # Write only known filenames, never a remote archive path.
            with archive.open(member) as source, (destination / filename).open("wb") as target:
                shutil.copyfileobj(source, target)
    if operating_system == "macos":
        os.chmod(destination / executable, 0o755)
    (destination / "VERSION.json").write_text(json.dumps({"version": VERSION, "asset": name, "sha256": digest}, indent=2) + "\n", encoding="utf-8")
    print(f"Installed {executable} ({VERSION}); archive SHA-256 {digest}")


if __name__ == "__main__":
    main()
