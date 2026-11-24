"""两遍响度标准化示例插件（process 入口，协议 2，格式转换器）。

只用 Python 标准库，转换交给软件自带的 FFmpeg（环境变量 MP_FFMPEG，
在 plugin.json 的 permissions.tools 里声明了 ffmpeg 才有）。

软件运行 `python main.py convert`，标准输入里的 params：
    input / output        输入文件、输出文件（必须写到 output）
    input_ext / output_ext
    options               转换页上用户选的参数（plugin.json 的 options）
    work_dir              临时文件夹，转换结束后软件会删掉
    duration              输入的时长（秒），读不出时是 null
"""
import json
import os
import re
import subprocess
import threading
import sys

# 输出扩展名 → 音频编码参数
AUDIO_CODECS = {
    "mp3": ["-c:a", "libmp3lame", "-b:a", "320k"],
    "flac": ["-c:a", "flac"],
    "m4a": ["-c:a", "aac", "-b:a", "256k"],
    "aac": ["-c:a", "aac", "-b:a", "256k"],
    "opus": ["-c:a", "libopus", "-b:a", "160k"],
    "ogg": ["-c:a", "libvorbis", "-q:a", "6"],
    "oga": ["-c:a", "libvorbis", "-q:a", "6"],
    "wav": ["-c:a", "pcm_s16le"],
    "aif": ["-c:a", "pcm_s16be"],
    "aiff": ["-c:a", "pcm_s16be"],
    "wv": ["-c:a", "wavpack"],
    "mka": ["-c:a", "flac"],
}
# 能保留封面的格式
COVER_OK = {"mp3", "flac", "m4a"}
VIDEO = {
    "mp4", "m4v", "mkv", "avi", "mov", "wmv", "flv", "webm", "ts", "m2ts", "mts",
    "rmvb", "rm", "3gp", "mpg", "mpeg", "vob", "ogv",
}
NO_WINDOW = 0x08000000 if os.name == "nt" else 0

def emit(event, **kw):
    kw["event"] = event
    sys.stdout.buffer.write((json.dumps(kw, ensure_ascii=False) + "\n").encode("utf-8"))
    sys.stdout.flush()

def fail(code, message):
    emit("error", code=code, message=message)
    sys.exit(0)

def ffmpeg():
    p = os.environ.get("MP_FFMPEG")
    if not p:
        fail("bad_config", "没有拿到 FFmpeg：plugin.json 的 permissions.tools 里要声明 ffmpeg")
    return p

def measure(ff, src, target, tp, lra, duration):
    """第一遍：测量，返回 loudnorm 的统计 JSON。进度占前一半"""
    af = f"loudnorm=I={target}:TP={tp}:LRA={lra}:print_format=json"
    cmd = [ff, "-hide_banner", "-nostats", "-progress", "pipe:1", "-i", src, "-vn", "-af", af, "-f", "null", "-"]
    err = run(cmd, duration, 0.0, 0.5)
    m = re.search(r"\{[^{}]*\"input_i\"[^{}]*\}", err, re.S)
    if not m:
        fail("internal", "第一遍测量没有拿到响度数据：" + err.strip().splitlines()[-1] if err.strip() else "第一遍测量失败")
    return json.loads(m.group(0))

def run(cmd, duration, base, span):
    """运行 FFmpeg，把 -progress 的 out_time 换成 0–1 的进度；返回标准错误全文"""
    p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, stdin=subprocess.DEVNULL, creationflags=NO_WINDOW)
    err_chunks = []

    t = threading.Thread(target=lambda: err_chunks.append(p.stderr.read()))
    t.start()
    for raw in p.stdout:
        line = raw.decode("utf-8", "replace").strip()
        if line.startswith("out_time_us=") and duration:
            try:
                sec = int(line.split("=", 1)[1]) / 1_000_000
            except ValueError:
                continue
            emit("progress", done=round(base + span * min(sec / duration, 1.0), 4), total=1)
    p.wait()
    t.join()
    err = b"".join(c for c in err_chunks if c).decode("utf-8", "replace")
    if p.returncode != 0:
        tail = [l for l in err.splitlines() if l.strip()][-3:]
        fail("internal", "FFmpeg 出错：" + " ".join(tail))
    return err

def convert(params):
    ff = ffmpeg()
    src, dst = params["input"], params["output"]
    in_ext, out_ext = params.get("input_ext", ""), params.get("output_ext", "")
    opt = params.get("options") or {}
    target = float(opt.get("target") or -16)
    tp = float(opt.get("true_peak") or -1.5)
    lra = int(opt.get("lra") or 11)
    rate = str(opt.get("sample_rate") or "keep")
    duration = params.get("duration")

    emit("log", msg=f"第一遍：测量 {os.path.basename(src)}")
    st = measure(ff, src, target, tp, lra, duration)
    emit("log", msg=f"原响度 {st['input_i']} LUFS，峰值 {st['input_tp']} dBTP")

    af = (
        f"loudnorm=I={target}:TP={tp}:LRA={lra}"
        f":measured_I={st['input_i']}:measured_TP={st['input_tp']}"
        f":measured_LRA={st['input_lra']}:measured_thresh={st['input_thresh']}"
        f":offset={st['target_offset']}:linear=true"
    )
    cmd = [ff, "-hide_banner", "-nostats", "-y", "-progress", "pipe:1", "-i", src, "-map_metadata", "0"]
    video_out = out_ext in VIDEO
    if video_out:
        # 视频：画面原样复制，只处理音轨
        cmd += ["-map", "0:v?", "-map", "0:a:0", "-c:v", "copy", "-af", af, "-c:a", "aac", "-b:a", "256k"]
    else:
        cmd += ["-map", "0:a:0"]
        if out_ext in COVER_OK and in_ext not in VIDEO:
            cmd += ["-map", "0:v?", "-c:v", "copy", "-disposition:v", "attached_pic"]
        cmd += ["-af", af] + AUDIO_CODECS.get(out_ext, [])
    # loudnorm 内部用 192 kHz，输出要把采样率改回来
    if rate == "keep":
        cmd += ["-ar", str(probe_rate(src) or 48000)]
    else:
        cmd += ["-ar", rate]
    cmd.append(dst)
    emit("log", msg="第二遍：按测量值调整")
    run(cmd, duration, 0.5, 0.5)
    if not os.path.isfile(dst) or os.path.getsize(dst) == 0:
        fail("internal", "FFmpeg 没有生成输出文件")
    emit("result", data={"output": dst})

def probe_rate(src):
    fp = os.environ.get("MP_FFPROBE")
    if not fp:
        return None
    try:
        out = subprocess.run(
            [fp, "-v", "error", "-select_streams", "a:0", "-show_entries", "stream=sample_rate", "-of", "csv=p=0", src],
            capture_output=True,
            timeout=30,
            creationflags=NO_WINDOW,
        ).stdout.decode().strip()
        return int(out.splitlines()[0]) if out else None
    except Exception:
        return None

def main():
    method = sys.argv[1] if len(sys.argv) > 1 else ""
    try:
        req = json.loads(sys.stdin.buffer.read().decode("utf-8") or "{}")
    except ValueError:
        fail("internal", "请求不是 JSON")
    params = req.get("params") or {}
    if method == "info":
        emit("result", data={"ok": bool(os.environ.get("MP_FFMPEG")), "message": "FFmpeg 可用" if os.environ.get("MP_FFMPEG") else "没有拿到 FFmpeg"})
    elif method == "convert":
        convert(params)
    else:
        fail("unsupported", f"不支持的方法：{method}")

if __name__ == "__main__":
    main()
