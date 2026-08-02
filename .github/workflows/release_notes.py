#!/usr/bin/python3

from io import TextIOWrapper

INPUT_FILE_NAME = "release-notes.md"
OUTPUT_FILE_NAME = "release-notes.md"
CATEGORIES = ["feat", "fix", "chore", "ci", "docs"]
CATEGORY_MAPPING = {"feat": "Features", "fix": "Fixes", "docs": "Documentation"}
NON_CAT_NAME = "Additional"
HASH_LINK_BASE = "https://github.com/HuppiFluppi/survey-tool-server/commit/"


# takes a filename/path and returns a list of tuples with category, component, message, hash
def read_release_notes(file: str) -> list[tuple[str | None, str | None, str, str]]:
    out = []
    with open(file, "r") as f:
        for l in f.readlines():
            (hash, _, text) = l.partition(" ")
            if not text:
                print(f"Line malformed: {l}")
                exit(1)
            (cat, _, msg) = text.partition(": ")
            if not msg:
                print(f"Line not in conventional commit format: {l}")
                msg = cat
                action = None
                component = None
            else:
                (action, _, component) = cat.partition("(")
                if not component:
                    component = None
                else:
                    component = component.rstrip(")")
            out.append((action, component, msg, hash))
    return out


# takes a list of notes (format from read_release_notes) and returns a dict with category as key and list of tuple as value
# sorts by component
def group_release_notes(notes: list[tuple[str | None, str | None, str, str]]) -> dict[str, list[tuple[str | None, str, str]]]:
    out = {}
    for n in notes:
        if not n[0] or n[0].lower() not in CATEGORIES:  # no category
            c = NON_CAT_NAME
        else:
            c = n[0]

        if c not in out:
            out[c] = []

        out[c].append((n[1], n[2], n[3]))

    for v in out.values():
        v.sort(key=lambda t: t[0] if t[0] else "")

    return out


# takes grouped notes and writes them to a file in markdown format
def write_release_notes(file: str, grouped_notes: dict[str, list[tuple[str | None, str, str]]]):
    with open(file, "w") as f:
        # make sure features comes first
        if features := grouped_notes.pop("feat", None):
            _list_notes(f, "feat", features)

        # make sure fixes comes next
        if fixes := grouped_notes.pop("fix", None):
            _list_notes(f, "fix", fixes)

        # remove general to put it last
        general = grouped_notes.pop(NON_CAT_NAME, None)

        # everything else
        for k, v in grouped_notes.items():
            _list_notes(f, k, v)

        # lastly, the general category
        if general:
            _list_notes(f, NON_CAT_NAME, general)


def _list_notes(file: TextIOWrapper, cat: str, items: list[tuple[str | None, str, str]]):
    c = CATEGORY_MAPPING.get(cat, cat.capitalize())
    file.write(f"### {c} ### \n")
    for i in items:
        if i[0]:
            file.write(f"- [{i[0]}] {i[1]} ([{i[2]}]({HASH_LINK_BASE}{i[2]})) \n")
        else:
            file.write(f"- {i[1]} ([{i[2]}]({HASH_LINK_BASE}{i[2]})) \n")
    file.write("\n")


if __name__ == "__main__":
    notes = read_release_notes(INPUT_FILE_NAME)
    print(f"Read {len(notes)} release lines")
    grouped = group_release_notes(notes)
    print(f"Grouped into {len(grouped)} categories")
    write_release_notes(OUTPUT_FILE_NAME, grouped)
    print("Written release notes \n - Done")
