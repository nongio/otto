# Plugins

Skills Otto ships for the agents that run on it. A plugin is a directory in the
Open Plugins shape — a `.claude-plugin/plugin.json` and a `skills/` directory
with one `SKILL.md` per skill — so an agent loads a skill when it is relevant
rather than carrying it in context the whole time.

```
plugins/
└── otto/
    ├── .claude-plugin/plugin.json
    └── skills/
        └── otto/
            ├── SKILL.md          the router: which page a request needs
            ├── references/
            │   ├── configure.md  changing settings, with a page per topic in configure/
            │   └── files.md      Files palette commands, with protocol.md and starter in files/
            └── scripts/
                └── files-command drafts a Files command and publishes it, executable
```

One skill, `otto`, disclosed progressively: an agent loads `SKILL.md` — a table
and a few rules — and reads only the page the request is about. Its
`allowed-tools` frontmatter lists the commands the pages run (the
`org.otto.Settings` calls, reading the skill's own pages, `files-command` and
editing its drafts), for agents that honour it. `files-command check` and
running a draft are left out on purpose: they execute what the agent wrote.

## Where they are installed

The Arch packages install `plugins/otto/` whole, to
`/usr/share/otto/plugins/otto/`, keeping the tree and the file modes — see
`package()` in `PKGBUILD`, `PKGBUILD-git` and `PKGBUILD-nightly-bin`, and the
tarball the first two install from, `scripts/packaging/make-arch-tarball.sh`.
`scripts/packaging/verify-install.sh` asserts it landed. The deb and the rpm do
not ship it yet.

otto-agentsd reads that directory at startup (`components/otto-agentsd/src/skills.rs`):
every session it creates is told what is there, and publishes the same list as
AHP customizations for its clients — which is how the launcher completes skill
names. A person's own plugins go under `$XDG_DATA_HOME/otto/plugins/`, searched
first, and `OTTO_AGENTS_PLUGINS` replaces the search path for trying one out. See
[docs/developer/agents.md](../../docs/developer/agents.md#the-skills-the-desktop-gives-an-agent).

Adding a file to a skill needs no packaging change; adding a plugin beside
`otto/` does not either, but it does need to be in the tarball list.

These are about *using* Otto. The development skills — driving a nested session,
the scene debugger, layer design — are a separate thing and live outside the
source tree.

## How these are written

The agent reading them may be a small model. Write for that:

- **Steps, numbered, in order.** Not principles to weigh up.
- **Whole commands, ready to copy.** Never a fragment the reader has to
  assemble. Every page opens with an `## Exact commands` block.
- **Tables that decide**: "it printed X → say Y", "error contains X → do Y".
- **Say what not to do**, in as many words, where getting it wrong is costly.
- Keep a page under ~150 lines. If it grows past that, it is two pages.

`references/configure.md` is split by topic — appearance, displays, keyboard, shortcuts,
trackpad, dock, tiling, lock screen, greeter, virtual outputs, the rest — so an
agent reads the one page a request is about and not the other eleven. `configure.md` is
that area's router; each reference page carries its area's settings table
*and* the file-only keys that belong with it. Every identifier the compositor
publishes lives in exactly one page.

Those tables come from a running compositor's `Describe`, which is the authority
for what settings exist. When `src/settings/schema.rs` gains or changes a row,
update the page that owns it — and if the new row belongs to no page, it needs
one, plus a line in the router.
