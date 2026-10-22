#!/usr/bin/env python3
"""Parse a SKILL.md frontmatter with a real YAML parser, as the CLI's binding does,
and require the description to disclose exactly the executable and profile digests
that the installer pins.  usage: check-skill-frontmatter.py SKILL.md install-intend.sh"""
import re, sys, yaml
t = open(sys.argv[1], encoding="utf-8").read()
assert t.startswith("---\n"), "no frontmatter block"
d = yaml.safe_load(t.split("---")[1])
assert isinstance(d.get("name"), str) and isinstance(d.get("description"), str), "name/description must be strings"
assert len(d["description"]) <= 1024, "description over 1024 characters"
s = open(sys.argv[2]).read()
# Listing policy 2.3, criterion 5: the description discloses the download-and-run behaviour; the exact
# digests live in the tree, in the script that performs the check.
pinned = set(re.findall(r'^(?:BINARY|DIGEST)_[A-Z0-9_]+="([0-9a-f]{64})"', s, re.M))
assert len(pinned) >= 9, f"the installer pins {len(pinned)} digests; expected archives, executables and the profile"
assert re.search(r"[Dd]ownloads and runs an external binary", d["description"]), "description must disclose the download-and-run behaviour"
assert "sha256" in d["description"] and "install-intend.sh" in d["description"], "description must say where the digests are"
version = re.search(r'^VERSION="([^"]+)"$', s, re.M).group(1); assert version in d["description"], "description must name the pinned release"
print(f"frontmatter ok: name={d['name']} description={len(d['description'])} chars; behaviour disclosed; {len(pinned)} digests pinned in the tree for {version}")
