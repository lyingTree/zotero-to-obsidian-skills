"""Test real frozen executable with system Python absent from PATH."""
from __future__ import annotations
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
DIST = ROOT.joinpath("dist", "z2o-binary")
PRIMARY = "literature-zotero2obsidian"


def main():
    packages = [path for path in DIST.iterdir() if path.is_dir()]
    if len(packages) != 1:
        raise ValueError("expected exactly one distribution")
    package = packages[0]
    launcher = package.joinpath(PRIMARY, "bin", "z2o.exe" if os.name == "nt" else "z2o")
    manifest = json.loads(package.joinpath("runtime_manifest.json").read_text(encoding="utf-8"))
    with tempfile.TemporaryDirectory(prefix="z2o-smoke-") as temporary:
        work = Path(temporary)
        isolated = work.joinpath("empty-path")
        isolated.mkdir()
        environment = dict(os.environ)
        for key in ("PYTHONPATH", "PYTHONHOME", "PYTHONSTARTUP", "VIRTUAL_ENV"):
            environment.pop(key, None)
        environment["PATH"] = str(isolated)
        def run(*args, success=True):
            result = subprocess.run([str(launcher), *map(str, args)], cwd=work, env=environment,
                                    capture_output=True, encoding="utf-8", errors="replace", timeout=240)
            if (result.returncode == 0) != success:
                raise RuntimeError(f"{args}: {result.returncode}\n{result.stdout}\n{result.stderr}")
            return result.stdout
        assert run("--version").strip() == manifest["version"]
        assert "note_pipeline.process_markdown" in json.loads(run("describe"))["operations"]
        assert json.loads(run("verify-package", package))["version"] == manifest["version"]
        request = work.joinpath("request.json")
        request.write_text(json.dumps({"schema_version": 1,
            "operation": "deterministic_stage_checks.author_token", "args": [["Smith", "Jones"]]}), encoding="utf-8")
        assert json.loads(run("invoke", request))["result"] == "Smith and Jones"
        request.write_text(json.dumps({"schema_version": 1, "operation": "os.system"}), encoding="utf-8")
        run("invoke", request, success=False)
        script = work.joinpath("task.py")
        script.write_text("""
from pathlib import Path
import importlib
import sys
import zipfile
import fitz
from PIL import Image
from pypdf import PdfReader
from pyzotero import zotero
from prepare_source_inventory import prepare_sources
from note_pipeline import process_markdown
from deterministic_stage_checks import audit_properties, build_properties
root = Path.cwd()
assert getattr(sys, 'frozen', False)
for folder in (Path(sys._MEIPASS).joinpath('z2o_bundle', 'literature-zotero2obsidian', 'scripts'),
               Path(sys._MEIPASS).joinpath('z2o_bundle', 'literature-translate-zh', 'scripts')):
    for source in sorted(folder.glob('*.py')):
        importlib.import_module(source.stem)
pdf = root.joinpath('source.pdf')
with fitz.open() as document:
    document.new_page().insert_text((72, 72), 'Synthetic source page.')
    document.save(pdf)
assert len(PdfReader(str(pdf)).pages) == 1
png = root.joinpath('image.png')
Image.new('RGB', (8, 8), 'white').save(png)
with Image.open(png) as image:
    assert image.size == (8, 8)
docx = root.joinpath('supplement.docx')
with zipfile.ZipFile(docx, 'w') as archive:
    archive.writestr('word/document.xml', '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Synthetic supplement.</w:t></w:r></w:p><w:sectPr/></w:body></w:document>')
    archive.writestr('word/_rels/document.xml.rels', '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>')
    archive.writestr('[Content_Types].xml', '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>')
inventory = prepare_sources({'main': pdf, 'si': docx}, root.joinpath('inventory'),
                            source_roles={'main': 'main', 'si': 'supplement'}, dpi=72)
assert inventory.is_file()
original = root.joinpath('original.md')
original.write_bytes(b'# Synthetic title\\n\\n## 1 Introduction\\nText.\\n')
candidate = root.joinpath('candidate.md')
candidate.write_bytes(build_properties(original.read_bytes(), item_type='thesis',
                       confirmed={'类型': '学位论文', '年份': 2024, '关键词': []}))
report = audit_properties(original, candidate, mode='new', item_type='thesis')
assert report['checks']['PROP-A08']['status'] == 'passed'
result = process_markdown(candidate, root.joinpath('processed'))
assert Path(result['directory']).joinpath('result.json').is_file()
print('FROZEN_SMOKE_OK')
""", encoding="utf-8")
        assert "FROZEN_SMOKE_OK" in run("run-script", script)
        target = work.joinpath("installed-skills")
        run("install", "--skills-root", target)
        installed = target.joinpath(PRIMARY, "bin", launcher.name)
        assert installed.is_file()
        assert len([p for p in target.iterdir() if p.joinpath("SKILL.md").is_file()]) == 14
        check = subprocess.run([str(installed), "--version"], cwd=work, env=environment,
                               capture_output=True, encoding="utf-8", timeout=240, check=True)
        assert check.stdout.strip() == manifest["version"]
        request.write_text(json.dumps({"schema_version": 1, "operation": "deterministic_stage_checks.author_token",
                                      "args": [["Smith", "Jones"]]}), encoding="utf-8")
        check = subprocess.run([str(installed), "invoke", str(request)], cwd=work, env=environment,
                               capture_output=True, encoding="utf-8", timeout=240, check=True)
        assert json.loads(check.stdout)["result"] == "Smith and Jones"
        skill = package.joinpath("literature-filename", "SKILL.md")
        original = skill.read_bytes()
        try:
            skill.write_bytes(original + b"\n")
            run("verify-package", package, success=False)
        finally:
            skill.write_bytes(original)
    print("Frozen runtime smoke checks passed.")


if __name__ == "__main__":
    main()
