import { readFileSync, writeFileSync } from 'node:fs';
import process from 'node:process';

const version = process.argv[2] ?? process.env.APP_VERSION;
const semverPattern =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/;

if (!version || !semverPattern.test(version)) {
  throw new Error(`Expected a valid SemVer version, received: ${version ?? '<missing>'}`);
}

function updateJson(path) {
  const contents = readFileSync(path, 'utf8');
  const value = JSON.parse(contents);
  if (typeof value.version !== 'string') {
    throw new Error(`${path} does not contain a top-level version`);
  }

  const updated = contents.replace(
    /^(\s*)"version":\s*"[^"]+"/m,
    `$1"version": "${version}"`
  );
  writeFileSync(path, updated);
}

function updatePackageSection(path, sectionMarker) {
  const contents = readFileSync(path, 'utf8');
  const sectionStart = contents.indexOf(sectionMarker);
  if (sectionStart === -1) {
    throw new Error(`${path} does not contain ${sectionMarker}`);
  }

  const nextSection = contents.indexOf('\n[', sectionStart + sectionMarker.length);
  const sectionEnd = nextSection === -1 ? contents.length : nextSection;
  const section = contents.slice(sectionStart, sectionEnd);
  if (!/^version = "[^"]+"$/m.test(section)) {
    throw new Error(`${path} package section does not contain a version`);
  }

  const updatedSection = section.replace(/^version = "[^"]+"$/m, `version = "${version}"`);

  writeFileSync(path, `${contents.slice(0, sectionStart)}${updatedSection}${contents.slice(sectionEnd)}`);
}

updateJson('package.json');
updateJson('src-tauri/tauri.conf.json');
updatePackageSection('src-tauri/Cargo.toml', '[package]');
updatePackageSection('src-tauri/Cargo.lock', '[[package]]\nname = "gametweaks"');

process.stdout.write(`Configured GameTweaks version ${version}\n`);
