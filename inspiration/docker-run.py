#!/usr/bin/env python3
# vi: set ts=4 sw=4 et : -*-  tab-width:4  c-basic-offset:4 indent-tabs-mode:nil -*-
# Copyright 2014-2018, Seamus Connor, seamushc@gmail.com

# This program is free software: you can redistribute it and/or modify it under
# the terms of the GNU General Public License as published by the Free Software
# Foundation, either version 3 of the License, or (at your option) any later
# version.
#
# This program is distributed in the hope that it will be useful, but WITHOUT
# ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
# FOR A PARTICULAR PURPOSE.  See the GNU General Public License for more
# details.
#
# You should have received a copy of the GNU General Public License along with
# this program.  If not, see <http://www.gnu.org/licenses/>.

# NOTE: this script works with both python 2 and 3, but it has only been tested
# with python 2.7 and python 3.6.  The container it launches must contain:
#   - Python 2.6.6+/3.5+
#   - sudo (not required, but this script drops perms before launching the user
#   command)

from __future__ import print_function
from shlex import split
import os, sys, re
import getpass
import pickle
import base64
import hashlib
import glob
import locale
import pprint
from functools import reduce
from subprocess import check_output, STDOUT
from pathlib import Path

try:
    from subprocess import Popen, check_call, CalledProcessError, DEVNULL
except ImportError:
    from subprocess import Popen, check_call, CalledProcessError

    DEVNULL = open(os.devnull, "w")

# locale.setlocale(locale.LC_ALL, "en_US.UTF-8")


def mkhostname(c):
    regex = re.compile("[^a-zA-Z0-9-]")
    c = c.split("/")[-1]
    return regex.sub("-", c)[:63]


def stream_file_to_sha(hashfn, fname):
    with open(fname, "rb") as f:
        while True:
            data = f.read(1 << 20)  # read up to 1 MB at a time
            if not data:
                break
            hashfn.update(data)


def rebuild_context_sha(filelist):
    m = hashlib.sha1()
    for f in filelist:
        if os.path.isfile(f):
            stream_file_to_sha(m, f)
    return m.hexdigest()


def shafile_is_dirty(shafile, filelist):
    # If the sha file does not exist, then it is dirty
    if not os.path.exists(shafile):
        return True

    # Read the existing sha file, and verify that the
    # file list matches
    with open(shafile) as f:
        shafile_contents = [x.strip() for x in f.readlines()]
    if len(shafile_contents) != len(filelist) + 1:
        return True
    if shafile_contents[1:] != filelist:
        return True

    shafile_mtime = os.path.getmtime(shafile)
    for f in filelist:
        if os.path.getmtime(f) > shafile_mtime:
            return True
    return False


def build_context_sha(shafile):
    filelist = []
    with open(".dockerignore") as f:
        try:
            # The first line must ignore everthing
            if next(f) != "*\n":
                raise Exception()

            for l in f:
                if l.startswith("!"):
                    filelist += glob.glob(l[1:].strip())
                else:
                    # All lines except for the first line must negate the ignore
                    raise Exception()
        except:
            print(
                "Error: docker ignore must start with '*', each following line must start with '!'"
            )
            sys.exit(1)
    filelist.append("Dockerfile")
    filelist.append(".dockerignore")
    for e in filelist:
        if os.path.isdir(e):
            for r, d, f in os.walk(e):
                filelist.extend(map(lambda x: os.path.join(r, x), f))
    filelist = list(filter(os.path.isfile, filelist))
    filelist.sort()

    if shafile_is_dirty(shafile, filelist):
        sha = rebuild_context_sha(filelist)
        with open(shafile, "w") as f:
            f.write(sha + "\n")
            f.write("\n".join(filelist))
        return sha
    else:
        with open(shafile) as f:
            return f.readline().strip()


# Handle decoding python 3 byte arrays for py2 compat
def decode(s):
    if sys.version_info.major == 2:
        return s
    return s.decode()


def encode(s):
    if sys.version_info.major == 2:
        return s
    return s.encode()


root_dir = Path("/")
cwd = Path(".").resolve()
orig_cwd = cwd
while cwd != root_dir:
    config_file = next(
        (
            cwd / x
            for x in (".docker_build_root", "docker_build_root")
            if (cwd / x).exists()
        ),
        None,
    )
    if config_file:
        break
    cwd = cwd.parent
else:
    print("Error: never found a config file", file=sys.stderr)
    sys.exit(-1)

config_file = str(config_file)
os.chdir(cwd)

# Save the directory containing the config file
rd = os.path.abspath(os.curdir)

env_overrides = {}
env_overrides["DR_BUILD_ROOT"] = rd

# Parse config file. Format is:
#   param whitespace list of arguments
with open(config_file) as f:
    params = {}
    for l in f:
        l = l.strip()
        if not l or l[0] == "#":
            continue
        l = split(l)
        if l:
            params[l[0]] = l[1:]

dr_uuid = params.get("uuid", [None])[0]
if dr_uuid:
    dr_uuid = dr_uuid.replace("-", "")


class EnvOpt:
    SET = 1
    ADD = 2
    DEL = 3


prefixes = {
    EnvOpt.SET: "DR_USER_OPT_SET_",
    EnvOpt.ADD: "DR_USER_OPT_ADD_",
    EnvOpt.DEL: "DR_USER_OPT_DEL_",
}

# Determine if an environment variable applies
# to this container.
def handle_env_opt(opt):
    prefix = None
    for k, v in prefixes.items():
        if opt.startswith(v):
            prefix = k
    if not prefix:
        return (None, None)

    opt = opt[len(prefixes[prefix]) :]
    if not opt.startswith("UUID_"):
        return (prefix, opt)
    if not dr_uuid:
        return (None, None)
    uuidprefix = "UUID_" + dr_uuid + "_"
    if not opt.startswith(uuidprefix):
        return (None, None)
    return (prefix, opt[len(uuidprefix) :])


# Use the environment to add/delete/override dr.py options
for v in os.environ:
    t, opt = handle_env_opt(v)
    if t == EnvOpt.DEL:
        if opt in params:
            del params[opt]
    elif t == EnvOpt.ADD:
        val = os.environ[v]
        if opt in params:
            params[opt] += split(val)
        else:
            params[opt] = split(val)
    elif t == EnvOpt.SET:
        val = os.environ[v]
        params[opt] = split(val)

# docker_container must be specified, all others are optional
if not "docker_container" in params:
    print("Error: docker_container must be specified in", config_file, file=sys.stderr)
    sys.exit(-1)
container = params["docker_container"][0]

# prefix_cmd and prefix_cmd_quiet should not coexist
if "prefix_cmd" in params and "prefix_cmd_quiet" in params:
    print(
        "Error: must specify at most one of prefix_cmd and prefix_cmd_quiet",
        file=sys.stderr,
    )
    sys.exit(-1)


ctx_sha = None
if "version_by_build_context" in params:
    ctx = params["version_by_build_context"]
    if len(ctx) != 1 or not os.path.exists(".dockerignore"):
        print(
            "Error: version_by_build_context requires a docker ignore file with negated patterns"
        )
        sys.exit(-1)
    ctx_sha = build_context_sha(ctx[0])

if not "extra_args" in params:
    params["extra_args"] = []

if ctx_sha:
    container += ":" + ctx_sha

do_print = False
do_rebuild = False
extra_args = []
while len(sys.argv) > 1:
    if not sys.argv[1].startswith("--dr-"):
        break

    if sys.argv[1] == "--dr-print":
        do_print = True
    elif sys.argv[1] == "--dr-ctx":
        if ctx_sha:
            print(ctx_sha)
            sys.exit(0)
        else:
            print("Error: context sha us unused by this configuration", file=sys.stderr)
            sys.exit(-1)
    elif sys.argv[1] == "--dr-print-image":
        print(container)
        sys.exit(0)
    elif sys.argv[1].startswith("--dr-use-ctx="):
        ctx_sha = sys.argv[1][len("--dr-use-ctx=") :]
    elif sys.argv[1].startswith("--dr-img="):
        container = sys.argv[1][len("--dr-img=") :]
    elif sys.argv[1] == "--dr-rebuild":
        do_rebuild = True
    elif sys.argv[1].startswith("--dr-extra-args="):
        extra_args.extend(split(sys.argv[1][len("--dr-extra-args="):]))
    elif sys.argv[1] == "--dr-show-config":
        pprint.pprint(params)
        sys.exit(0)
    elif sys.argv[1] == "--dr-help":
        print(
            """
DR Flags:
    print: print the docker command instead of executing it
    ctx: print the context sha
    print-image: print the image
    use-ctx: force a particular context sha
    img: force a particular image
    rebuild: rebuild the docker image
    show-config: dump the parameters
    extra-args: add extra args to the docker invocation
"""
        )
        sys.exit(0)

    del sys.argv[1]

if "prelaunch_hook" in params:
    check_call(params["prelaunch_hook"])

if do_rebuild:
    print("Rebuilding container", container)
    try:
        check_call(["docker", "build", "-t", container, os.curdir])
    except CalledProcessError:
        sys.exit(1)


# build the command to pass through docker
usr_cmd = sys.argv[1:]

# If -- is in the args, then send all of the args before the -- to docker, and
# the rest to the user command
if "--" in sys.argv:
    idx = sys.argv.index("--")
    params["extra_args"] += sys.argv[1:idx]
    usr_cmd = sys.argv[idx + 1 :]

# By default, share the root directory at the same path within the container
shared_dir = "{}:{}".format(rd, rd)
cd_to = str(orig_cwd)
if "mount_to" in params:
    shared_dir = "{}:{}".format(rd, params["mount_to"][0])
    cd_to = params["mount_to"][0]

if "cd_to" in params:
    cd_to = params["cd_to"][0]

command = ["docker", "run", "-i", "--rm", "--init", "-v", shared_dir]

# Run interactively if the user is currently interactive.
terminfo = ""
if os.isatty(sys.stdout.fileno()) and os.isatty(sys.stdin.fileno()):
    env_overrides["TERM"] = os.environ["TERM"]
    terminfo = (
        '"'
        + decode(base64.b64encode(check_output(["infocmp", os.environ["TERM"]])))
        + '"'
    )
    command.append("-t")

command.append("--privileged=true")
# command.append("--cap-add=SYS_PTRACE")
# command.append("--security-opt seccomp=unconfined")

if "extra_hosts" in params:
    for h in params["extra_hosts"]:
        command.append("--add-host={}".format(h))

# Add a -v /share/path:/share/path for each extra_share. If an extra share is
# of the form "$<NAME>", then it is expanded as an environment variable.
if "extra_shares" in params:
    shares = (
        os.environ.get(x[1:]) if x.startswith("$") else x
        for x in params["extra_shares"]
    )
    shares = (x for x in shares if x)
    shares = (["-v", x + ":" + x] if ":" not in x else ["-v", x] for x in map(os.path.abspath, shares))
    for s in shares:
        command += s

# Since git supports moving the git dir out of the tree, optionally add an
# extra share for the real git-dir location in this case.
if "share_git_dir" in params:
    try:
        git_dir = (
            check_output(["git", "rev-parse", "--git-common-dir"], stderr=DEVNULL)
            .decode()
            .strip()
        )
        git_dir = os.path.abspath(git_dir)
        if not git_dir.startswith(rd):
            command += ["-v", git_dir + ":" + git_dir]
    except CalledProcessError:
        # Not a git repo, but that is OK.
        pass

command += extra_args
if len(params["extra_args"]):
    command += params["extra_args"]

persist_environment = None
if "persist_environment" in params:
    persist_environment = os.path.realpath(params["persist_environment"][0])

if "env_overrides" in params:
    for o in params["env_overrides"]:
        if o in os.environ:
            env_overrides[o] = os.environ[o]

def mk_bash_exe_env(cmds):
    return "{ " + " ".join(cmds) + "; }"


cmds = []
if "extra_shell" in params:
    p = str(Path(params["extra_shell"][0]).resolve())
    command += ["-v", p + ":" + p]
    cmds.append(f"source {p}")
if "prefix_cmd" in params:
    cmds.append(mk_bash_exe_env(params["prefix_cmd"]) + " < /dev/null")
elif "prefix_cmd_quiet" in params:
    cmds.append(
        mk_bash_exe_env(params["prefix_cmd_quiet"]) + " < /dev/null > /dev/null 2>&1"
    )


if len(usr_cmd):
    cmds.append(mk_bash_exe_env(usr_cmd))
    cmds.append("drrc=$?")

if persist_environment:
    # This command is injected into the subshell after the user command to siphon off the updated environment
    # Dont save off SHLVL because it will increment up forever
    save_environment = encode(
        """
import os
import pickle

env = dict(os.environ)
del env['SHLVL']
with open('{}', 'wb') as f:
    pickle.dump(env, f, protocol=2)
""".format(
            persist_environment
        )
    )

    # Encode the command so that shell escaping isn't a total nightmare
    save_environment = decode(base64.b64encode(save_environment))
    cmds.append(
        "python3 -c \\'"
        + 'import base64; exec(base64.b64decode("{}"))'.format(save_environment)
        + "\\'"
    )

# The subshell should exit with the return code of the user command
if len(usr_cmd):
    cmds.append("exit $drrc")

cmd_format = {
    "restore_env": "True" if persist_environment else "False",
    "env_file": persist_environment if persist_environment else "",
    "cd_to": cd_to,
    "root_dir": rd,
    "user": getpass.getuser(),
    "uid": os.getuid(),
    "gid": os.getgid(),
    "env_overrides": decode(base64.b64encode(pickle.dumps(env_overrides, protocol=2))),
    "usr_cmd": ";".join(cmds),
    "terminfo": terminfo,
    "has_terminfo": str(bool(terminfo)),
    "extra_shell": params.get("extra_shell", ""),
}

# The code inside this string is run inside the context of the container.  As
# such, all containers accessed in this way must have a recentish version of
# python. This has been tested on python 2.6.6+ and 3.6+
internal_process = """
from __future__ import print_function
import os
import pickle
import pwd
import subprocess
import sys
import base64

# Change directories to what appears to be the cwd of the invoker
os.chdir('{cd_to}')

# Create a user with identical uid/gid to invoker
home = '/tmp/dr-tmp-home-{user}'
if not os.path.exists(home):
    os.makedirs(home)
os.chmod(home, 0o755)
os.system('userdel os76')
os.system('groupadd -g {gid} {user}')
os.system('useradd -d /' + home + '/{user} -m -g {gid} -u {uid} {user}')
os.system('sed -ir "s/.*{user}.*//g" /etc/sudoers')

# Give sudo access
with open('/etc/sudoers', 'a') as f:
    f.write('{user} ALL=(ALL) NOPASSWD: ALL')

env = os.environ # default to the current env
if {restore_env}:
    if os.path.exists('{env_file}'):
        with open('{env_file}', 'rb') as f:
            env = pickle.load(f)

env.update(pickle.loads(base64.b64decode(b'{env_overrides}')))
env['HOME'] = home + '/{user}'

os.setgid({gid})
os.setuid({uid})
cmd = r'''{usr_cmd}'''
shell = '/bin/bash'
if not os.path.exists(shell):
    shell = '/bin/sh'
try:
    if {has_terminfo}:
        subprocess.check_output(['mkdir', '-p', env['HOME'] + '/.terminfo'])
        with open(env['HOME'] + '/terminfo', 'wb') as f:
            f.write(base64.b64decode({terminfo}))
        subprocess.check_output(['tic', env['HOME'] + '/terminfo'], env=env)
except subprocess.CalledProcessError:
    pass
try:
    os.execvpe(shell, [shell, '-c', cmd], env)
except OSError:
    print('Error: failed to launch command.')
""".format(
    **cmd_format
)

command += [
    "-h",
    mkhostname(container),  # name the host after the container
    "-u", "root",
    container,
    "/usr/bin/env",
    "python3",
    "-c",
    internal_process,
]

cmd_len = reduce(lambda x, y: x + y, map(lambda x: len(x), command))
cmd_lim = int(check_output(["getconf", "ARG_MAX"]))
if cmd_len > cmd_lim:
    print(
        "Error: command is too long for this system.",
        file=sys.stderr,
    )
    exit(-1)

if do_print:
    for c in command:
        print("++++", c)
else:
    os.execvp("docker", command)
