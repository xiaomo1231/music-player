# 两遍响度标准化（格式转换插件示例）

装好后在“格式转换”页的“插件”一行出现“响度标准化 → 原格式 / FLAC / MP3 320k”。

- 第一遍用 FFmpeg 的 loudnorm 测量整首的响度、峰值和响度范围；第二遍按测量值线性调整，结果比内置的单遍“响度均衡”更准，不会出现前后音量忽大忽小。
- 参数：目标响度（-14 / -16 / -23 LUFS）、峰值上限、响度范围、采样率。
- 输入：软件认识的音频和视频格式；视频只处理音轨，画面原样复制。
- 用的是软件自带的 FFmpeg（`permissions.tools` 里声明了 `ffmpeg`、`ffprobe`），只用 Python 标准库。

本地试一次：

```
python tools/mpplugin_check.py tools/plugin-examples/loudness-normalize --convert 某首歌.wav --to flac --opt target=-14
```
