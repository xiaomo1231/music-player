"""插件检查工具：检查 plugin.json（规则和软件里的 manifest.rs 一致），并按协议 2 实际调用一遍 process 插件。

用法：
    python tools/mpplugin_check.py <插件文件夹或 .mpx> [--query 关键字] [--set key=value ...]
                                   [--source 源 id] [--kind music] [--download] [--app-version 0.6.0]
                                   [--convert 文件 [--to 扩展名] [--converter id] [--opt key=value ...] [--out 文件]]

- process 入口（协议 2）：依次调用 info、search、episodes、resolve（不下载），加 --download 时把第一条
  结果实际 download 到临时文件夹，检查返回值的格式。
- 格式转换器（contributes.converters）：加 --convert 时把这个文件实际转换一次（协议 2 的 convert 方法，
  或 exec 命令模板），输出到临时文件夹，--out 可以另存一份。
- js / rule 入口：只检查清单（js 在软件的 QuickJS 沙箱里运行，请用软件里插件详情的“测试”页）。

只用 Python 标准库。工具本身不联网（插件自己联网除外）。退出码：0 没有错误，1 有错误。
说明见 docs/插件开发.md。
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import zipfile

KINDS = {"music", "video", "anime", "manga", "novel"}
CAPS = {"search", "episodes", "resolve", "download"}
SETTING_TYPES = {"string", "secret", "bool", "int", "select", "folder"}
TOOLS = {"ytdlp": "yt-dlp", "ffmpeg": "ffmpeg", "ffprobe": "ffprobe"}
TARGETS = {"http", "ytdlp", "magnet", "pages", "chapters", "file"}
ROLES = {"main", "subtitle", "lyric", "cover"}
# 软件认的错误码（fetch/provider.rs 的 ERROR_CODES）。不在里面的会被当成 internal
ERROR_CODES = {"login_required", "captcha", "forbidden", "not_found", "rate_limited",
               "network", "bad_config", "unsupported", "internal", "plugin_disabled"}
INTERPRETERS = {"python", "python3", "py", "node", "deno", "bun", "pwsh", "powershell", "cmd", "sh", "bash"}
TARGET_HEADERS = {"referer", "accept", "accept-language", "authorization"}
YTDLP_FORMAT_RE = re.compile(r"^[A-Za-z0-9*+/\[\]<>=!:._,\-?^$|()]{0,120}$")
# 和软件一样的时限：总时长（秒），download 另有 300 秒没有输出就停
TIMEOUT = {"info": 15, "search": 60, "episodes": 60, "resolve": 120, "download": 24 * 3600, "convert": 24 * 3600}
OPTION_TYPES = {"string", "bool", "int", "select"}
INPUT_GROUPS = {"audio", "video", "*"}
EXEC_VARS = {"input", "output", "output_dir", "input_ext", "output_ext", "work_dir", "plugin_dir", "ffmpeg", "ffprobe"}
AUDIO = {"mp3", "flac", "m4a", "aac", "alac", "wav", "ape", "wv", "ogg", "oga", "opus", "wma", "aif", "aiff",
         "dsf", "dff", "tta", "tak", "ac3", "dts", "mka"}
VIDEO = {"mp4", "m4v", "mkv", "avi", "mov", "wmv", "flv", "webm", "ts", "m2ts", "mts", "rmvb", "rm", "3gp",
         "mpg", "mpeg", "vob", "ogv"}
EXT_RE = re.compile(r"^[a-z0-9]{1,16}$")
IDLE = {"download": 300}

problems = []
warnings = []


def err(msg):
    problems.append(msg)
    print(f"  ✗ {msg}")


def warn(msg):
    warnings.append(msg)
    print(f"  ! {msg}")


def ok(msg):
    print(f"  ✓ {msg}")


# ---------- 清单 ----------

def id_ok(s, min_len):
    return (isinstance(s, str) and min_len <= len(s) <= 64 and re.fullmatch(r"[a-z0-9-]+", s or "") is not None
            and not s.startswith("-") and not s.endswith("-"))


def parse_version(v):
    if not isinstance(v, str):
        return None
    core = re.split(r"[-+]", v.strip(), maxsplit=1)[0]
    parts = core.split(".")
    if len(parts) != 3 or not all(p.isdigit() for p in parts):
        return None
    return tuple(int(p) for p in parts)


def safe_rel(p):
    if not isinstance(p, str) or not p:
        return False
    if p.startswith(("/", "\\")) or re.match(r"^[A-Za-z]:", p):
        return False
    return ".." not in re.split(r"[/\\]", p)


def obj(v):
    return v if isinstance(v, dict) else {}


def lst(v):
    return v if isinstance(v, list) else []


def check_manifest(root, app_version):
    path = os.path.join(root, "plugin.json")
    try:
        with open(path, "rb") as f:
            raw = f.read()
    except FileNotFoundError:
        err("没有 plugin.json")
        return None
    if len(raw) > 256 * 1024:
        err("plugin.json 太大（上限 256 KB）")
    try:
        m = json.loads(raw.decode("utf-8").lstrip("﻿"))
    except UnicodeDecodeError:
        err("plugin.json 要用 UTF-8 编码")
        return None
    except json.JSONDecodeError as e:
        err(f"plugin.json 第 {e.lineno} 行格式不对：{e.msg}")
        return None
    if not isinstance(m, dict):
        err("plugin.json 要是一个对象")
        return None

    if not m.get("manifest"):
        err("缺少 manifest（写 1）")
    elif m.get("manifest") != 1:
        err(f"这个插件需要更新版本的软件（清单格式 {m.get('manifest')}）")
    if not id_ok(m.get("id"), 3):
        err("id 只能用小写字母、数字、-，3–64 位，不能以 - 开头或结尾")
    if not str(m.get("name") or "").strip():
        err("缺少 name")
    if parse_version(m.get("version")) is None:
        err("version 要写成 主.次.修，如 1.0.0")

    eng = obj(m.get("engines"))
    app = str(eng.get("app") or "").strip()
    if app and app != "*":
        need = parse_version(app.lstrip(">=^").strip())
        if need is None:
            err(f"engines.app 写成 >=主.次.修，如 >=0.6.0：{app}")
        elif app_version and parse_version(app_version) and parse_version(app_version) < need:
            err(f"这个插件需要 {app} 版本的软件，当前是 {app_version}")
    protocol = eng.get("protocol") or 0

    rt = obj(m.get("runtime"))
    rtype = rt.get("type")
    c = obj(m.get("contributes"))
    converters = [obj(x) for x in lst(c.get("converters"))]
    exec_only = not lst(c.get("sources")) and converters and all(lst(x.get("exec")) for x in converters)
    if rtype == "process" and exec_only and not obj(rt.get("command")):
        pass
    elif rtype == "process":
        cmd = obj(rt.get("command"))
        for k in cmd:
            if k not in ("windows", "macos", "linux", "default"):
                err(f"runtime.command 不认识的系统：{k}")
        argv = command_for(m)
        if not argv:
            err("runtime.command 要按系统写命令数组，如 {\"default\": [\"python\", \"main.py\"]}")
        else:
            a0 = argv[0]
            if os.path.isabs(a0) or re.match(r"^[A-Za-z]:", a0):
                err("runtime.command 不能写绝对路径，用相对插件文件夹的路径")
            looks_local = ("/" in a0 or "\\" in a0 or os.path.splitext(a0)[1])
            stem = os.path.splitext(os.path.basename(a0))[0].lower()
            if looks_local and stem not in INTERPRETERS and not os.path.isfile(os.path.join(root, a0)):
                err(f"runtime.command 里的程序不存在：{a0}")
            for a in argv[1:]:
                if os.path.splitext(a)[1] and safe_rel(a) and not os.path.exists(os.path.join(root, a)):
                    warn(f"runtime.command 里的文件不在插件文件夹里：{a}")
    elif rtype in ("rule", "js"):
        f = rt.get("file") or ""
        if not safe_rel(f) or not os.path.isfile(os.path.join(root, f)):
            err(f"runtime.file 指向的文件不存在：{f}")
    else:
        err("runtime.type 只能是 rule / process / js")
    if (rtype == "js" or (rtype == "process" and not exec_only)) and protocol not in (1, 2):
        if isinstance(protocol, int) and protocol > 2:
            err(f"这个插件用的是更新的协议 {protocol}，请先升级软件")
        else:
            err("process / js 入口要写 engines.protocol（2）")
    if rtype == "js" and protocol != 2:
        err("js 入口只支持协议 2")
    if rtype == "process" and protocol == 1:
        warn("协议 1 是旧协议，新插件请写 engines.protocol: 2")

    sources = lst(c.get("sources"))
    rules = lst(c.get("rules"))
    if not sources and not rules and not converters:
        err("contributes 里至少要有 sources、rules 或 converters")
    if rtype == "rule" and len(sources) > 1:
        err("rule 入口只能有一个下载源")
    seen = set()
    for i, s in enumerate(sources):
        s = obj(s)
        w = f"contributes.sources[{i}]"
        sid = s.get("id", "")
        if not id_ok(sid, 1):
            err(f"{w}.id 只能用小写字母、数字、-")
        if sid in seen:
            err(f"{w}.id 重复：{sid}")
        seen.add(sid)
        bad = [k for k in lst(s.get("media_kinds")) if k not in KINDS]
        if bad:
            err(f"{w}.media_kinds 不认识：{bad}")
        caps = lst(s.get("capabilities"))
        bad = [k for k in caps if k not in CAPS]
        if bad:
            err(f"{w}.capabilities 不认识：{bad}")
        if "search" not in caps:
            err(f"{w}.capabilities 必须有 search")
        if "resolve" not in caps and "download" not in caps:
            err(f"{w}.capabilities 至少要有 resolve 或 download")
        if rtype != "process" and "download" in caps:
            err(f"{w}：rule / js 入口不能 download（没有文件权限），请用 resolve")
        if protocol == 1 and ("episodes" in caps or "download" in caps):
            err(f"{w}：协议 1 只支持 search / resolve")
        if "resolve" in caps and "download" in caps:
            warn(f"{w}：同时有 resolve 和 download 时软件只用 resolve，download 不会被调用")
    for r in rules:
        if not safe_rel(r) or not os.path.isfile(os.path.join(root, r)):
            err(f"contributes.rules 里的文件不存在：{r}")

    p = obj(m.get("permissions"))
    check_converters(m, root, rtype, converters, lst(p.get("tools")))
    login = p.get("login") or "none"
    hosts = lst(p.get("hosts"))
    if login not in ("none", "optional", "required"):
        err("permissions.login 只能是 none / optional / required")
    if login != "none" and not hosts:
        err("用登录（login）时必须写 permissions.hosts")
    bad = [t for t in lst(p.get("tools")) if t not in TOOLS]
    if bad:
        err(f"permissions.tools 只能是 ytdlp / ffmpeg / ffprobe：{bad}")
    settings = [obj(s) for s in lst(m.get("settings"))]
    keys = [s.get("key") for s in settings]
    for x in lst(p.get("paths")):
        if not (isinstance(x, str) and x.startswith("setting:") and x[8:] in keys):
            err(f"permissions.paths 只能引用设置项（setting:<key>）：{x}")
    for h in hosts:
        if not isinstance(h, str) or "://" in h or "/" in h or not h.strip():
            err(f"permissions.hosts 只写域名（可带端口和 *.），不写 http:// 和路径：{h}")
    seen = set()
    for i, s in enumerate(settings):
        if not s.get("key") or not s.get("label"):
            err(f"settings[{i}] 要有 key 和 label")
        if s.get("key") in seen:
            err(f"settings[{i}].key 重复：{s.get('key')}")
        seen.add(s.get("key"))
        if s.get("type") not in SETTING_TYPES:
            err(f"settings[{i}].type 只能是 string / secret / bool / int / select / folder")
        if s.get("type") == "select" and not s.get("options"):
            err(f"settings[{i}] 是 select，要写 options")
        if s.get("type") == "secret" and "default" in s:
            warn(f"settings[{i}] 是 secret，default 不会生效（密钥只从系统凭据管理器读）")
    icon = m.get("icon") or ""
    if icon and (not safe_rel(icon) or not os.path.isfile(os.path.join(root, icon))):
        err(f"icon 文件不存在：{icon}")
    for x in lst(m.get("legacy_ids")):
        if not (isinstance(x, str) and (x.startswith("ext-") or x.startswith("em-"))):
            err(f"legacy_ids 只能写 ext- 或 em- 开头的旧下载源 id：{x}")

    if rtype == "rule" and not problems:
        check_rule_hosts(root, rt.get("file"), hosts)

    if not problems:
        conv = f"，{len(converters)} 个转换器" if converters else ""
        ok(f"plugin.json：{m['name']} {m['version']}（{rtype}，{len(sources)} 个下载源{conv}）")
    return m


def output_list(cv):
    """转换器的输出格式 [(ext, name)]；写错的项返回 None"""
    out = []
    for o in lst(cv.get("outputs")):
        if isinstance(o, str):
            out.append((o.strip().lstrip(".").lower(), ""))
        elif isinstance(o, dict) and isinstance(o.get("ext"), str):
            out.append((o["ext"].strip().lstrip(".").lower(), str(o.get("name") or "")))
        else:
            out.append(None)
    return out


def exec_vars(arg):
    """命令模板一项里的占位符；{{ 和 }} 是字面的花括号"""
    out, i = [], 0
    while i < len(arg):
        two = arg[i:i + 2]
        if two in ("{{", "}}"):
            i += 2
        elif arg[i] == "{":
            j = arg.find("}", i + 1)
            if j < 0:
                raise ValueError(f"花括号没有闭合：{arg}")
            out.append(arg[i + 1:j])
            i = j + 1
        elif arg[i] == "}":
            raise ValueError(f"多了一个 }}：{arg}")
        else:
            i += 1
    return out


def fill_exec(arg, values):
    out, i = [], 0
    while i < len(arg):
        two = arg[i:i + 2]
        if two == "{{":
            out.append("{")
            i += 2
        elif two == "}}":
            out.append("}")
            i += 2
        elif arg[i] == "{":
            j = arg.find("}", i + 1)
            name = arg[i + 1:j]
            if name not in values:
                raise ValueError(f"占位符 {{{name}}} 没有值")
            out.append(values[name])
            i = j + 1
        else:
            out.append(arg[i])
            i += 1
    return "".join(out)


def accepts(inputs, ext):
    e = ext.lower()
    return any(i == "*" or (i == "audio" and e in AUDIO) or (i == "video" and e in VIDEO) or (e and i == e)
               for i in inputs)


def check_converters(m, root, rtype, converters, tools):
    if converters and rtype != "process":
        err("converters 只能用在 process 入口（转换要读写文件）")
    setting_keys = {obj(s).get("key") for s in lst(m.get("settings"))}
    seen = set()
    for i, cv in enumerate(converters):
        w = f"contributes.converters[{i}]"
        cid = cv.get("id", "")
        if not id_ok(cid, 1):
            err(f"{w}.id 只能用小写字母、数字、-")
        if cid in seen:
            err(f"{w}.id 重复：{cid}")
        seen.add(cid)
        inputs = lst(cv.get("inputs"))
        if not inputs:
            err(f"{w}.inputs 至少写一种输入格式（扩展名，或 audio / video / *）")
        for x in inputs:
            if not (x in INPUT_GROUPS or (isinstance(x, str) and EXT_RE.match(x))):
                err(f"{w}.inputs 只能写不带点的小写扩展名，或 audio / video / *：{x}")
        outs = output_list(cv)
        if not outs:
            err(f"{w}.outputs 至少写一种输出格式")
        if any(o is None for o in outs):
            err(f'{w}.outputs 每一项是扩展名字符串，或 {{"ext": "mp3", "name": "…"}}')
        exts = [o[0] for o in outs if o]
        for e in exts:
            if not (e == "same" or EXT_RE.match(e)):
                err(f"{w}.outputs 的扩展名只能用小写字母和数字（或 same）：{e}")
        for e in {e for e in exts if exts.count(e) > 1}:
            err(f"{w}.outputs 重复：{e}")
        okeys = set()
        for j, o in enumerate(lst(cv.get("options"))):
            o = obj(o)
            if not o.get("key") or not o.get("label"):
                err(f"{w}.options[{j}] 要有 key 和 label")
            if o.get("key") in okeys:
                err(f"{w}.options[{j}].key 重复：{o.get('key')}")
            okeys.add(o.get("key"))
            if o.get("type") not in OPTION_TYPES:
                err(f"{w}.options[{j}].type 只能是 string / bool / int / select")
            if o.get("type") == "select" and not o.get("options"):
                err(f"{w}.options[{j}] 是 select，要写 options")
        ex = [str(a) for a in lst(cv.get("exec"))]
        if not ex:
            continue
        a0 = ex[0]
        if os.path.isabs(a0) or re.match(r"^[A-Za-z]:", a0) or a0.startswith(("/", "\\")):
            err(f"{w}.exec 不能写绝对路径，用相对插件文件夹的路径")
        elif not a0.startswith("{"):
            looks_local = "/" in a0 or "\\" in a0 or os.path.splitext(a0)[1]
            if looks_local and (not safe_rel(a0) or not os.path.isfile(os.path.join(root, a0))):
                err(f"{w}.exec 里的程序不存在：{a0}")
        has_out = False
        for a in ex:
            try:
                names = exec_vars(a)
            except ValueError as e:
                err(f"{w}.exec：{e}")
                continue
            for v in names:
                if v in ("output", "output_dir"):
                    has_out = True
                if v in ("ffmpeg", "ffprobe") and v not in tools:
                    err(f"{w}.exec 用了 {{{v}}}，要在 permissions.tools 里声明 {v}")
                if v.startswith("option:"):
                    if v[7:] not in okeys:
                        err(f"{w}.exec 引用的参数不存在：{{{v}}}")
                elif v.startswith("setting:"):
                    if v[8:] not in setting_keys:
                        err(f"{w}.exec 引用的设置项不存在：{{{v}}}")
                elif v not in EXEC_VARS:
                    err(f"{w}.exec 不认识的占位符：{{{v}}}")
        if not has_out:
            err(f"{w}.exec 要用 {{output}}（或 {{output_dir}}）告诉程序输出到哪里")


def check_rule_hosts(root, file, hosts):
    """rule 入口：.em 里的 hosts 要和 permissions.hosts 一致（有 PyYAML 时才检查）"""
    try:
        import yaml  # noqa: 可选
    except ImportError:
        print("  · 没装 PyYAML，跳过 .em 的 hosts 对比（安装时软件会检查）")
        return
    try:
        with open(os.path.join(root, file), encoding="utf-8") as f:
            rule = yaml.safe_load(f)
    except Exception as e:  # noqa
        err(f"{file} 读不了：{e}")
        return
    rule = obj(rule)
    if rule.get("kind") != "source":
        err("rule 入口要指向 kind: source 的规则")
    if sorted(lst(rule.get("hosts"))) != sorted(hosts):
        err("plugin.json 的 permissions.hosts 要和 .em 里的 hosts 一致")
    if (rule.get("login") or "none") != "none" and not hosts:
        err(".em 用了 login，plugin.json 也要写 hosts")


# ---------- 调用 ----------

def command_for(m):
    cmd = obj(obj(m.get("runtime")).get("command"))
    key = {"win32": "windows", "darwin": "macos"}.get(sys.platform, "linux")
    argv = cmd.get(key) or cmd.get("default")
    return [str(a) for a in argv] if isinstance(argv, list) and argv else None


class Runner:
    def __init__(self, root, m, app_version, data_dir):
        self.root, self.m = root, m
        self.argv = command_for(m) or []
        a0 = os.path.basename(self.argv[0]).lower() if self.argv else ""
        if a0 in ("python", "python3", "py", "python.exe", "python3.exe", "py.exe"):
            if shutil.which(self.argv[0]) is None:
                warn(f"PATH 里找不到 {self.argv[0]}，用 {sys.executable} 代替（用户电脑上也要有它，"
                     "或者按系统分别写 runtime.command）")
            self.argv = [sys.executable] + self.argv[1:]
        elif self.argv:
            local = os.path.join(root, self.argv[0])
            if os.path.isfile(local):
                self.argv = [local] + self.argv[1:]
        self.env = dict(os.environ, MP_PROVIDER_PROTOCOL="2", PYTHONIOENCODING="utf-8", PYTHONUTF8="1",
                        MP_PLUGIN_ID=m["id"], MP_PLUGIN_DIR=root, MP_PLUGIN_DATA=data_dir,
                        MP_TEMP=tempfile.gettempdir(), MP_LANG="zh-CN", MP_APP_VERSION=app_version)
        for t in lst(obj(m.get("permissions")).get("tools")):
            exe = shutil.which(TOOLS.get(t, t))
            if exe:
                self.env[f"MP_{t.upper()}"] = exe
            else:
                warn(f"本机 PATH 里没有 {TOOLS.get(t, t)}，MP_{t.upper()} 不会设置（软件里会指向自带的）")
        self.context = {"app_version": app_version, "lang": "zh-CN", "cookies": {}}

    def call(self, method, params, settings, on_progress=None):
        """按协议 2 调一次，返回 (result, error_dict)。"""
        req = {"protocol": 2, "method": method, "params": params, "settings": settings, "context": self.context}
        t0 = time.time()
        try:
            p = subprocess.Popen(self.argv + [method], cwd=self.root, env=self.env, stdin=subprocess.PIPE,
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        except OSError as e:
            return None, {"code": "internal", "message": f"启动失败：{e}（命令 {self.argv}）"}
        try:
            p.stdin.write(json.dumps(req, ensure_ascii=False).encode("utf-8"))
            p.stdin.close()
        except OSError:
            pass
        stderr_lines = []
        t_err = threading.Thread(target=lambda: stderr_lines.extend(p.stderr.read().decode("utf-8", "replace").splitlines()))
        t_err.daemon = True
        t_err.start()
        lines = []
        last_out = [time.time()]

        def read_out():
            for raw in p.stdout:
                last_out[0] = time.time()
                lines.append(raw)
        t_out = threading.Thread(target=read_out)
        t_out.daemon = True
        t_out.start()

        result, error, last_event, seen = None, None, None, 0
        killed = None
        while True:
            t_out.join(0.2)
            while seen < len(lines):
                line = lines[seen].decode("utf-8", "replace").strip().lstrip("﻿")
                seen += 1
                if not line:
                    continue
                if not line.startswith("{"):
                    print(f"      日志 {line[:300]}")
                    continue
                try:
                    ev = json.loads(line)
                except json.JSONDecodeError:
                    print(f"      日志 {line[:300]}")
                    continue
                last_event = ev.get("event")
                if last_event == "log":
                    print(f"      日志[{ev.get('level', 'info')}] {ev.get('msg', '')}")
                elif last_event == "progress":
                    if not isinstance(ev.get("done"), (int, float)):
                        err(f"{method}：progress 事件的 done 要是数字")
                    if on_progress:
                        on_progress(ev)
                elif last_event == "result":
                    result, error = ev.get("data"), None
                elif last_event == "error":
                    error = ev
                else:
                    warn(f"{method}：不认识的事件 {last_event}（软件会当作日志）")
            if not t_out.is_alive() and seen >= len(lines):
                break
            if time.time() - t0 > TIMEOUT[method]:
                killed = f"{method} 超过 {TIMEOUT[method]} 秒没有结束"
                break
            if method in IDLE and time.time() - last_out[0] > IDLE[method]:
                killed = f"{IDLE[method]} 秒没有任何输出"
                break
        if killed:
            p.kill()
            return None, {"code": "internal", "message": killed}
        try:
            p.wait(10)
        except subprocess.TimeoutExpired:
            p.kill()
        t_err.join(2)
        tail = [l for l in stderr_lines if l.strip()][-20:]
        for l in tail:
            print(f"      stderr {l[:300]}")
        if result is None and error is None:
            if p.returncode not in (0, None) and tail:
                error = {"code": "internal", "message": tail[-1]}
            elif p.returncode not in (0, None):
                error = {"code": "internal", "message": f"插件出错（退出码 {p.returncode}）"}
            else:
                error = {"code": "internal", "message": "插件没有输出 result 或 error 事件"}
        elif last_event not in ("result", "error"):
            warn(f"{method}：最后一个事件应当是 result 或 error")
        if error and error.get("code") not in ERROR_CODES:
            err(f"{method}：错误码 {error.get('code')} 不在约定里，软件会当成 internal")
        print(f"      用时 {time.time() - t0:.2f} 秒")
        return result, error


# ---------- 返回值检查（和软件里 serde 读取的规则一致） ----------

def is_int(v):
    return isinstance(v, int) and not isinstance(v, bool)


def is_num(v):
    return isinstance(v, (int, float)) and not isinstance(v, bool)


ITEM_STR = ("id", "title", "artist", "album", "cover", "media_kind", "series")
ITEM_INT = ("year", "track", "season")
ITEM_NUM = ("duration", "episode")


def check_item(it, kind, w):
    for k in ITEM_STR:
        if k in it and not isinstance(it[k], str):
            err(f"{w}.{k} 要是字符串（不要写 null，没有就不写）")
    for k in ITEM_INT:
        if it.get(k) is not None and not is_int(it[k]):
            err(f"{w}.{k} 要是整数或 null")
    for k in ITEM_NUM:
        if it.get(k) is not None and not is_num(it[k]):
            err(f"{w}.{k} 要是数字或 null")
    qs = it.get("qualities", [])
    if not isinstance(qs, list):
        err(f"{w}.qualities 要是数组")
        qs = []
    for j, q in enumerate(qs):
        if not isinstance(q, dict):
            err(f"{w}.qualities[{j}] 要是对象")
            continue
        for k in ("key", "label", "format"):
            if k in q and not isinstance(q[k], str):
                err(f"{w}.qualities[{j}].{k} 要是字符串")
        if q.get("bitrate") is not None and not (is_int(q["bitrate"]) and q["bitrate"] >= 0):
            err(f"{w}.qualities[{j}].bitrate 要是非负整数")
        if q.get("size") is not None and not is_int(q["size"]):
            err(f"{w}.qualities[{j}].size 要是整数（字节）")
        if not q.get("key") or not q.get("label"):
            warn(f"{w}.qualities[{j}] 最好有 key 和 label")
    mk = it.get("media_kind") or kind
    if mk not in KINDS:
        err(f"{w}.media_kind 不认识：{mk}")
    if not str(it.get("id") or "").strip():
        warn(f"{w} 没有 id（同一条每次搜索都应当是同一个 id）")
    if mk == "music" and not it.get("artist"):
        warn(f"{w}：音乐最好给 artist（整理入库按艺人归档）")
    if mk in ("anime", "manga", "novel") and not it.get("series"):
        warn(f"{w}：番剧 / 漫画 / 小说最好给 series（搜索页按它分组）")


def check_items(v, kind, where):
    items = v.get("items") if isinstance(v, dict) else v
    if not isinstance(items, list):
        err(f"{where} 要返回数组或 {{\"items\": [...]}}")
        return []
    out, ids = [], set()
    for i, it in enumerate(items):
        w = f"{where}[{i}]"
        if not isinstance(it, dict):
            err(f"{w} 不是对象")
            continue
        check_item(it, kind, w)
        if not str(it.get("title") or "").strip():
            warn(f"{w} 没有 title，软件会跳过这条")
            continue
        if it.get("id") and it["id"] in ids:
            warn(f"{w}.id 重复：{it['id']}")
        ids.add(it.get("id"))
        out.append(it)
    return out


def host_allowed(url, hosts):
    m = re.match(r"^(https?)://(?:[^@/]*@)?(\[[^\]]+\]|[^/:?#]+)(?::(\d+))?", url or "", re.I)
    if not m:
        return False, "不是 http / https 网址"
    name, port = m.group(2).lower().strip("[]").rstrip("."), m.group(3)
    literal = re.fullmatch(r"[\d.]+", name) or ":" in name
    special = literal or name == "localhost" or name.endswith(".localhost")
    for h in hosts:
        h = h.strip().lower()
        wild = h.startswith("*.")
        rest = h[2:] if wild else h
        hp = None
        if ":" in rest and rest.rsplit(":", 1)[1].isdigit():
            rest, hp = rest.rsplit(":", 1)
        if hp != port:
            continue
        if wild and not special and name.endswith("." + rest):
            return True, ""
        if not wild and name == rest:
            return True, ""
    return False, f"网站 {name}{':' + port if port else ''} 不在 permissions.hosts 里，软件会拒绝下载"


def check_resolved(res, hosts, rtype):
    if not isinstance(res, dict) or not isinstance(res.get("target"), dict):
        err("resolve 要返回 {\"target\": {...}, \"lyric\": ...}")
        return
    if res.get("lyric") is not None and not isinstance(res["lyric"], str):
        err("lyric 要是字符串（LRC 全文）或 null")
    t = res["target"]
    ty = t.get("type")
    if ty not in TARGETS:
        err(f"target.type 只能是 {sorted(TARGETS)}")
        return
    need = {"http": ("url", "ext"), "ytdlp": ("url",), "magnet": ("uri",), "pages": ("urls",),
            "chapters": ("chapters",), "file": ("path",)}[ty]
    for k in need:
        if k not in t:
            err(f"target（{ty}）缺少 {k}")
    urls = []
    if ty in ("http", "ytdlp"):
        urls = [t.get("url", "")]
    elif ty == "pages":
        urls = lst(t.get("urls"))
        if not urls:
            err("pages 没有图片地址")
    elif ty == "chapters":
        chs = lst(t.get("chapters"))
        if not chs:
            err("chapters 没有章节")
        for i, c in enumerate(chs):
            if not isinstance(c, dict) or not isinstance(c.get("title", ""), str) or not isinstance(c.get("text", ""), str):
                err(f"chapters[{i}] 要是 {{\"title\": 字符串, \"text\": 字符串}}")
    elif ty == "file":
        if rtype != "process":
            err("只有 process 入口可以返回 file")
        elif not os.path.isfile(str(t.get("path", ""))):
            err(f"file 指向的文件不存在：{t.get('path')}")
        else:
            warn("resolve 返回 file 是协议 1 的旧写法，协议 2 请用 download 能力")
    for u in urls:
        allowed, why = host_allowed(u, hosts)
        if not allowed:
            err(f"下载地址 {u[:120]}：{why}")
    if ty in ("http", "pages"):
        for k in obj(t.get("headers")):
            if k.lower() not in TARGET_HEADERS:
                err(f"target.headers 不能有 {k}（只能 Referer、Accept、Accept-Language、Authorization）")
    if ty == "ytdlp" and t.get("format") and not (YTDLP_FORMAT_RE.match(t["format"]) and not t["format"].startswith("-")):
        err(f"yt-dlp 格式选择不正确：{t['format']}")
    ok(f"resolve：{ty} {(urls[0][:100] if urls else '')}")


def check_download(res, dest):
    files = res.get("files") if isinstance(res, dict) else None
    if not isinstance(files, list) or not files:
        err("download 要返回 {\"files\": [{\"path\": ..., \"role\": \"main\"}]}")
        return
    root = os.path.realpath(dest)
    mains = 0
    for f in files:
        if not isinstance(f, dict) or not isinstance(f.get("path"), str):
            err("files 的每一项要有 path")
            continue
        role = f.get("role", "main")
        if role not in ROLES:
            err(f"role 不认识：{role}（只能 main / subtitle / lyric / cover）")
        mains += role == "main"
        p = os.path.realpath(f["path"])
        if os.path.commonpath([root, p]) != root:
            err(f"文件不在 dest_dir 里：{f['path']}")
        elif not os.path.isfile(p):
            err(f"返回的文件不存在：{f['path']}")
    if not mains:
        err("files 里至少要有一个 role 为 main 的文件")
    ok(f"download：{[os.path.basename(f.get('path', '')) + '（' + f.get('role', 'main') + '）' for f in files if isinstance(f, dict)]}")


# ---------- 主流程 ----------

def unpack(path):
    tmp = tempfile.mkdtemp(prefix="mpx-")
    with zipfile.ZipFile(path) as z:
        names = z.namelist()
        if len(names) > 5000:
            raise ValueError("插件包里的文件太多（上限 5000 个）")
        total = 0
        for info in z.infolist():
            n = info.filename.replace("\\", "/")
            if n.startswith("/") or ".." in n.split("/") or re.match(r"^[A-Za-z]:", n):
                raise ValueError(f"插件包里有不安全的路径：{n}")
            if (info.external_attr >> 16) & 0o170000 == 0o120000:
                raise ValueError(f"插件包里不能有符号链接：{n}")
            total += info.file_size
        if total > 200 << 20:
            raise ValueError("插件包解压后太大（上限 200 MB）")
        z.extractall(tmp)
    if os.path.isfile(os.path.join(tmp, "plugin.json")):
        return tmp, tmp
    subs = [d for d in os.listdir(tmp) if not d.startswith("__MACOSX")]
    if len(subs) == 1 and os.path.isfile(os.path.join(tmp, subs[0], "plugin.json")):
        return tmp, os.path.join(tmp, subs[0])
    raise ValueError("插件包里没有 plugin.json")


def run(a):
    root = os.path.abspath(a.plugin)
    if os.path.isfile(root):
        if not zipfile.is_zipfile(root):
            err("不是插件包：要选插件文件夹（里面有 plugin.json）或 .mpx 文件")
            return
        try:
            tmp, root = unpack(root)
        except (ValueError, zipfile.BadZipFile) as e:
            err(str(e))
            return
        a.cleanup.append(tmp)

    print(f"检查 {a.plugin}")
    m = check_manifest(root, a.app_version)
    if m is None or problems:
        return

    rtype = m["runtime"]["type"]
    if rtype == "js":
        print("  · js 入口在软件的沙箱里运行，这里只检查清单。请在软件里“设置 → 插件 → 开发者 → 加载插件文件夹…”，"
              "再到插件详情的“测试”页试")
        return
    if rtype == "rule":
        print("  · rule 入口只检查清单。.em 规则可以在“设置 → 规则文件”里导入后用规则的测试面板试")
        return
    has_cmd = bool(command_for(m))
    if has_cmd and obj(m.get("engines")).get("protocol") != 2:
        print("  · 协议 1 插件只检查清单")
        return

    settings = {}
    for s in [obj(x) for x in lst(m.get("settings"))]:
        if s.get("type") != "secret" and "default" in s:
            settings[s["key"]] = s["default"]
        else:
            settings[s.get("key")] = None
    types = {s.get("key"): s.get("type") for s in [obj(x) for x in lst(m.get("settings"))]}
    for kv in a.set:
        k, _, v = kv.partition("=")
        if k not in types:
            warn(f"--set {k}：plugin.json 里没有这个设置项")
        t = types.get(k)
        if t == "int":
            try:
                v = int(v)
            except ValueError:
                err(f"--set {k} 要是整数")
                continue
        elif t == "bool":
            v = v.lower() in ("1", "true", "yes", "on")
        elif t == "folder" and v and not os.path.isdir(v):
            err(f"--set {k}：找不到文件夹 {v}")
        settings[k] = v
    missing = [s["key"] for s in lst(m.get("settings"))
               if isinstance(s, dict) and s.get("required") and (settings.get(s.get("key")) in (None, ""))]
    if missing:
        err(f"必填设置没给：{missing}（用 --set {missing[0]}=...）。软件里会直接报 bad_config，不运行插件")
        return

    data_dir = tempfile.mkdtemp(prefix="mp-data-")
    a.cleanup.append(data_dir)
    runner = Runner(root, m, a.app_version or "0.6.0", data_dir)
    hosts = lst(obj(m.get("permissions")).get("hosts"))
    sources = m["contributes"].get("sources") or []
    if a.source:
        sources = [s for s in sources if s.get("id") == a.source]
        if not sources:
            err(f"没有 id 为 {a.source} 的下载源")
            return
    converters = [obj(x) for x in lst(m["contributes"].get("converters"))]
    if converters:
        run_convert(a, runner, root, m, converters, settings)
    for src in sources:
        caps = set(src.get("capabilities") or [])
        kind = a.kind or (src.get("media_kinds") or ["music"])[0]
        sid = src["id"]
        print(f"\n下载源 {sid}（{kind}）")
        print("  info")
        res, e = runner.call("info", {"source": sid}, settings)
        if e and e.get("code") == "unsupported":
            warn("info：插件没有实现 info（软件安装后和测试时会调用它做自检，建议实现）")
        elif e:
            err(f"info：[{e.get('code')}] {e.get('message')}")
            continue
        else:
            ok(f"info：{json.dumps(res, ensure_ascii=False)[:200]}")
        if not a.query:
            print("  · 没给 --query，跳过搜索")
            continue
        print(f"  search “{a.query}”")
        res, e = runner.call("search", {"source": sid, "query": a.query, "media_kind": kind, "page": 1}, settings)
        if e:
            err(f"search：[{e.get('code')}] {e.get('message')}")
            continue
        items = check_items(res, kind, "search")
        ok(f"search：{len(items)} 条")
        for it in items[:5]:
            print(f"      {it.get('title')} · {it.get('artist', '')} · {it.get('album', '')}  id={it.get('id')}")
        if not items:
            continue
        first = items[0]
        if "episodes" in caps:
            work = next((it for it in items if it.get("episode") is None), None)
            if work:
                print("  episodes")
                res, e = runner.call("episodes", {"source": sid, "item": work}, settings)
                if e:
                    err(f"episodes：[{e.get('code')}] {e.get('message')}")
                else:
                    eps = check_items(res, kind, "episodes")
                    ok(f"episodes：{len(eps)} 集")
                    if eps:
                        first = eps[0]
        quality = (lst(first.get("qualities")) or [{}])[0].get("key", "")
        if "resolve" in caps:
            print("  resolve")
            res, e = runner.call("resolve", {"source": sid, "item": first, "quality": quality}, settings)
            if e:
                err(f"resolve：[{e.get('code')}] {e.get('message')}")
            else:
                check_resolved(res, hosts, "process")
        elif "download" in caps and a.download:
            print("  download")
            dest = tempfile.mkdtemp(prefix="mp-dl-")
            a.cleanup.append(dest)
            last = {}
            res, e = runner.call("download", {"source": sid, "item": first, "quality": quality,
                                              "dest_dir": dest, "resume": False}, settings, last.update)
            if last:
                print(f"      最后进度 {last.get('done')}/{last.get('total')} {last.get('unit', '')}")
            if e:
                err(f"download：[{e.get('code')}] {e.get('message')}")
            else:
                check_download(res, dest)
        elif "download" in caps:
            print("  · 加 --download 实际下载第一条到临时文件夹")


def run_convert(a, runner, root, m, converters, settings):
    if a.converter:
        converters = [c for c in converters if c.get("id") == a.converter]
        if not converters:
            err(f"没有 id 为 {a.converter} 的转换器")
            return
    if command_for(m) and not all(lst(c.get("exec")) for c in converters):
        print("\n转换器自检 info")
        res, e = runner.call("info", {}, settings)
        if e and e.get("code") != "unsupported":
            err(f"info：[{e.get('code')}] {e.get('message')}")
        elif not e:
            ok(f"info：{json.dumps(res, ensure_ascii=False)[:200]}")
    if not a.convert:
        names = "、".join(c.get("name") or c.get("id", "") for c in converters)
        print(f"  · 转换器：{names}。加 --convert 文件 实际转换一次")
        return
    src = os.path.abspath(a.convert)
    if not os.path.isfile(src):
        err(f"找不到要转换的文件：{a.convert}")
        return
    in_ext = os.path.splitext(src)[1].lstrip(".").lower()
    cv = next((c for c in converters if accepts(lst(c.get("inputs")), in_ext)), None)
    if cv is None:
        err(f"没有转换器支持 .{in_ext} 文件")
        return
    outs = [o for o in output_list(cv) if o]
    spec = (a.to or outs[0][0]).lstrip(".").lower()
    if spec not in [o[0] for o in outs]:
        err(f"转换器 {cv.get('id')} 不能输出 {spec}，可选：{[o[0] for o in outs]}")
        return
    out_ext = in_ext if spec == "same" else spec
    opts = {}
    for o in [obj(x) for x in lst(cv.get("options"))]:
        opts[o.get("key")] = o.get("default")
    otypes = {obj(o).get("key"): obj(o).get("type") for o in lst(cv.get("options"))}
    for kv in a.opt:
        k, _, v = kv.partition("=")
        if k not in otypes:
            warn(f"--opt {k}：转换器没有这个参数")
        if otypes.get(k) == "int":
            try:
                v = int(v)
            except ValueError:
                err(f"--opt {k} 要是整数")
                return
        elif otypes.get(k) == "bool":
            v = v.lower() in ("1", "true", "yes", "on")
        opts[k] = v
    work = tempfile.mkdtemp(prefix="mp-conv-")
    a.cleanup.append(work)
    out_dir = os.path.join(work, "out")
    os.makedirs(out_dir)
    dst = os.path.join(work, "result" + (f".{out_ext}" if out_ext else ""))
    print(f"\n转换器 {cv.get('id')}：{os.path.basename(src)} → {out_ext or '（无扩展名）'}")
    t0 = time.time()
    ex = [str(x) for x in lst(cv.get("exec"))]
    if ex:
        values = {"input": src, "output": dst, "output_dir": out_dir, "input_ext": in_ext, "output_ext": out_ext,
                  "work_dir": work, "plugin_dir": root,
                  "ffmpeg": runner.env.get("MP_FFMPEG", ""), "ffprobe": runner.env.get("MP_FFPROBE", "")}
        for k, v in opts.items():
            values[f"option:{k}"] = "" if v is None else str(v).lower() if isinstance(v, bool) else str(v)
        for k, v in settings.items():
            values[f"setting:{k}"] = "" if v is None else str(v).lower() if isinstance(v, bool) else str(v)
        try:
            argv = [fill_exec(x, values) for x in ex]
        except ValueError as e:
            err(f"exec：{e}")
            return
        if not ex[0].startswith("{") and ("/" in ex[0] or "\\" in ex[0] or os.path.splitext(ex[0])[1]):
            argv[0] = os.path.join(root, argv[0])
        print(f"      运行 {argv}")
        try:
            r = subprocess.run(argv, cwd=work, env=runner.env, capture_output=True, timeout=TIMEOUT["convert"])
        except OSError as e:
            err(f"exec：启动失败：{e}")
            return
        tail = [l for l in r.stderr.decode("utf-8", "replace").splitlines() if l.strip()][-6:]
        for l in tail:
            print(f"      stderr {l[:300]}")
        if r.returncode != 0:
            err(f"exec：程序出错（退出码 {r.returncode}）")
            return
    else:
        params = {"converter": cv.get("id"), "input": src, "output": dst, "input_ext": in_ext, "output_ext": out_ext,
                  "options": opts, "work_dir": work, "duration": None}
        last = {}
        res, e = runner.call("convert", params, settings, last.update)
        if last:
            print(f"      最后进度 {last.get('done')}/{last.get('total', 1)}")
        if e:
            err(f"convert：[{e.get('code')}] {e.get('message')}")
            return
        given = obj(res).get("output") if isinstance(res, dict) else None
        if given and os.path.abspath(given) != dst:
            if not os.path.abspath(given).startswith(work) or not os.path.isfile(given):
                err("convert：返回的 output 不在软件给的位置（output 或 work_dir 里）")
                return
            shutil.move(given, dst)
    if not os.path.isfile(dst):
        files = [os.path.join(out_dir, f) for f in os.listdir(out_dir)]
        files = [f for f in files if os.path.isfile(f)]
        hit = files if len(files) == 1 else [f for f in files if f.lower().endswith("." + out_ext)]
        if hit:
            shutil.move(hit[0], dst)
    if not os.path.isfile(dst) or os.path.getsize(dst) == 0:
        err("转换没有生成输出文件（要写到 params.output / {output}，或 {output_dir} 里）")
        return
    ok(f"转换完成：{os.path.getsize(dst)} 字节，用时 {time.time() - t0:.1f} 秒")
    if a.out:
        shutil.copyfile(dst, a.out)
        print(f"      已另存到 {a.out}")


def main():
    ap = argparse.ArgumentParser(description="检查播放器插件（plugin.json + process 协议 2）")
    ap.add_argument("plugin", help="插件文件夹或 .mpx")
    ap.add_argument("--query", default="", help="搜索关键字；不给只调用 info")
    ap.add_argument("--kind", default="", help="搜索的媒体类型，默认用下载源的第一个 media_kinds")
    ap.add_argument("--source", default="", help="只检查这个下载源 id")
    ap.add_argument("--set", action="append", default=[], metavar="key=value", help="设置项的值，可以写多次")
    ap.add_argument("--download", action="store_true", help="实际调用 download（下到临时文件夹，用完删掉）")
    ap.add_argument("--app-version", default="", help="按这个软件版本检查 engines.app，如 0.6.0")
    ap.add_argument("--convert", default="", help="用转换器实际转换这个文件")
    ap.add_argument("--to", default="", help="输出扩展名（outputs 里的一项，可以是 same），默认第一个")
    ap.add_argument("--converter", default="", help="只用这个转换器 id")
    ap.add_argument("--opt", action="append", default=[], metavar="key=value", help="转换参数，可以写多次")
    ap.add_argument("--out", default="", help="把转换结果另存到这个文件")
    a = ap.parse_args()
    a.cleanup = []
    try:
        run(a)
    finally:
        for d in a.cleanup:
            shutil.rmtree(d, ignore_errors=True)
    tail = f"，{len(warnings)} 个提醒" if warnings else ""
    print(f"\n{'全部通过' if not problems else str(len(problems)) + ' 个问题'}{tail}")
    return 1 if problems else 0


if __name__ == "__main__":
    try:
        sys.stdout.reconfigure(encoding="utf-8")
    except (AttributeError, ValueError):
        pass
    sys.exit(main())
