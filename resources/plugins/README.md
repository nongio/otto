# Plugins

Skills Otto ships for the agents that run on it, and an agent of its own. A
plugin is a directory in the Open Plugins shape — a `.claude-plugin/plugin.json`
and a `skills/` directory with one `SKILL.md` per skill — so an agent loads a
skill when it is relevant rather than carrying it in context the whole time. An
`agents/` directory beside it holds agent files in the Claude Code plugin
shape: frontmatter (`name`, `description`, `tools`, `skills`, optionally
`model`) and a system prompt as the body.

```
plugins/
└── otto/
    ├── .claude-plugin/plugin.json
    ├── agents/
    │   └── otto.md               Otto's own helper: a delegate for desktop questions, working from the skill
    └── skills/
        └── otto-help/
            ├── SKILL.md          the router: which page a request needs
            ├── references/
            │   ├── configure.md  changing settings, with a page per topic in configure/
            │   ├── files.md      Files palette commands, with protocol.md and starter in files/
            │   └── docs/         docs/user copied in by scripts/sync-skill-docs.sh
            └── scripts/
                └── files-command drafts a Files command and publishes it, executable
```

One skill, `otto-help`, disclosed progressively: an agent loads `SKILL.md` — a table
and a few rules — and reads only the page the request is about. Its
`allowed-tools` frontmatter lists the commands the pages run (the
`org.otto.Settings` calls, reading the skill's own pages, `files-command` and
editing its drafts), for agents that honour it. `files-command check` and
running a draft are left out on purpose: they execute what the agent wrote.

One agent, `otto`, whose frontmatter names that skill: the session's agent
runs as it, or hands it a desktop question or a setting change, and it answers
from the skill's pages, in Otto's voice. Claude Code loads it from the plugin
directory; `otto-agents plugins install` renders the same file for OpenCode,
Hermes, Codex and pi, each in its own dialect and place, with the body
unchanged — this file is the only one to edit. It is read by five harnesses,
so it must not name one: say "the coding agent you run on", not Claude Code.

## Where they are installed

The Arch packages install `plugins/otto/` whole, to
`/usr/share/otto/plugins/otto/`, keeping the tree and the file modes — see
`package()` in `PKGBUILD`, `PKGBUILD-git` and `PKGBUILD-nightly-bin`, and the
tarball the first two install from, `scripts/packaging/make-arch-tarball.sh`.
`scripts/packaging/verify-install.sh` asserts it landed. The deb and the rpm
list the files by glob in the root `Cargo.toml` (`package.metadata.deb.assets`
and `package.metadata.generate-rpm.assets`), one line per directory.

otto-agents reads that directory at startup (`components/otto-agents/src/skills.rs`):
every session it creates is told what is there, and publishes the same list as
AHP customizations for its clients — which is how the launcher completes skill
names. A person's own plugins go under `$XDG_DATA_HOME/otto/plugins/`, searched
first, and `OTTO_AGENTS_PLUGINS` replaces the search path for trying one out. See
[docs/developer/agents.md](../../docs/developer/agents.md#the-skills-the-desktop-gives-an-agent).

otto-agents also reads `agents/*.md` and publishes each as an agent
customization under its plugin, so a client can see the delegate a session was
given. `otto-agents plugins install` renders each agent file for the
harnesses that cannot load it as it is (`components/otto-agents/src/vendors.rs`),
and `plugins status` shows where each copy stands.

Adding a file to a skill or an agent file to `agents/` needs no packaging
change for the Arch packages; the deb and rpm asset lists in the root
`Cargo.toml` glob `agents/*.md` alongside the skill files. Adding a plugin
beside `otto/` needs a place in the tarball list.

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

## The user guides travel with the skill

`references/docs/` is `docs/user/` copied in whole by
`scripts/sync-skill-docs.sh`, so the agent reads Otto's actual guides rather
than a paraphrase that drifted from them. `--check` fails when a copy is
stale and runs in CI's `locales` job; run the script and commit the result
after changing anything in `docs/user/`.

Copies rather than symlinks, because the packages install the plugin as a
plain directory and an agent reads it from `/usr/share`. The duplication is
real but mechanical: nothing is written twice by hand, and CI will not let
the two drift.

The guides are the long answer. The hand-written pages stay short and keep
what the guides deliberately lack — the exact `busctl` lines, the type
letters, the error tables. An agent reads a guide only when the short page
does not answer the question, and gives the person the documentation site,
`https://nongio.github.io/otto/<name>/`, rather than the local path or a
GitHub one. The slug is the guide's file name without `.md`, which is how
`website/build-docs.sh` publishes it; a page it does not list has no URL, so
check there before linking a new one.

Those tables come from a running compositor's `Describe`, which is the authority
for what settings exist. When `src/settings/schema.rs` gains or changes a row,
update the page that owns it — and if the new row belongs to no page, it needs
one, plus a line in the router.
