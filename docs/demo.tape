# Optional live-terminal capture (VHS).
# Primary hero GIF is generated without VHS:
#   python3 scripts/gen_demo_gif.py  →  docs/assets/demo.gif
#
# This tape is for a real shell recording:
#   brew install vhs
#   vhs docs/demo.tape
#
# Best demo: leave `heim -i 5` running while an agent edits.

Output docs/assets/demo.gif
Set FontSize 14
Set Width 1100
Set Height 620
Set Theme "Catppuccin Mocha"
Set Padding 12
Set WindowBar Colorful
Set TypingSpeed 40ms

# Build first if needed (comment out when using a release binary).
# Hide
# Type "cargo build --release" Enter
# Wait@60s /Finished/
# Show

Type "clear" Enter
Type "# Stop vibe-code bloat — live Δ + JSON agents can self-audit" Enter
Type "heim --once ." Enter
Sleep 2s
Type "heim --once --json . | head -n 40" Enter
Sleep 3s
Type "# Start the TUI beside your coding agent:" Enter
Type "heim -i 10" Enter
Sleep 4s
Escape
Sleep 500ms
Type "q"
Sleep 1s
