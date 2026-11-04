#!/usr/bin/env python3
"""Launch the selected modded game without opening the launcher window."""
import subprocess, sys
from fork import fork_launcher
subprocess.run([fork_launcher(), "play", "--instance", "horizon", *sys.argv[1:]], check=True)
