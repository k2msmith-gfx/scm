# Run a command in a hidden pseudo-terminal of a fixed size, reading and
# discarding what it writes (as a terminal would), until it exits.
import os, pty, sys, fcntl, termios, struct, select, time
rows, cols = int(sys.argv[1]), int(sys.argv[2])
cmd = sys.argv[3:]
pid, fd = pty.fork()
if pid == 0:
    os.environ["TERM"] = "xterm-256color"
    os.environ["COLORTERM"] = "truecolor"
    os.environ["TERM_PROGRAM"] = "iTerm.app"
    os.execvp(cmd[0], cmd)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
os.kill(pid, 28)  # SIGWINCH
deadline = time.time() + 180
total = 0
while time.time() < deadline:
    r, _, _ = select.select([fd], [], [], 0.5)
    if r:
        try:
            data = os.read(fd, 1 << 16)
        except OSError:
            break
        if not data:
            break
        total += len(data)
    if os.waitpid(pid, os.WNOHANG)[0]:
        break
else:
    os.kill(pid, 9)
    print("TIMEOUT")
print("bytes from the terminal:", total)
