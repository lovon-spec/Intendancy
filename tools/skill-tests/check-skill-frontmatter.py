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
pinned = set(re.findall(r'^BINARY_[A-Z0-9_]+="([0-9a-f]{64})"', s, re.M)) | set(re.findall(r'^DIGEST_PROFILE="([0-9a-f]{64})"', s, re.M))
disclosed = set(re.findall(r"sha256:([0-9a-f]{64})", d["description"]))
assert pinned and disclosed == pinned, f"disclosed != pinned: {sorted(disclosed ^ pinned)}"
print(f"frontmatter ok: name={d['name']} description={len(d['description'])} chars, {len(disclosed)} digests disclosed and pinned")
