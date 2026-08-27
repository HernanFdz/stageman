@AGENTS.md

<!--
An import, not a prose pointer: Claude Code loads the referenced file rather
than relying on the agent choosing to follow a sentence. Not a symlink either —
those break on Windows checkouts with core.symlinks=false.

Claude-specific instructions can go below this line without polluting the
portable file. Keep that to a minimum: anything that must hold for every harness
belongs in AGENTS.md, and anything that must be *enforced* belongs in the gate.
-->
