# Nix 2.35.2 `internal-json` 进度协议调查

调查对象是 Nix 2.35.2 tag 对应的官方源码提交
[`2c73b59da29606068c0c98db015dd3a66955525d`](https://github.com/NixOS/nix/tree/2c73b59da29606068c0c98db015dd3a66955525d)。

## 稳定性

- Nix 官方在 2.4 发布说明中将 `internal-json` 定义为“供其他程序使用”的日志格式：[`rl-2.4.md` 第 221–223 行](https://github.com/NixOS/nix/blob/2c73b59da29606068c0c98db015dd3a66955525d/doc/manual/source/release-notes/rl-2.4.md#L221-L223)。
- 同一发布说明仍将 `nix` 命令的接口标为 experimental：[`rl-2.4.md` 第 23–28 行](https://github.com/NixOS/nix/blob/2c73b59da29606068c0c98db015dd3a66955525d/doc/manual/source/release-notes/rl-2.4.md#L23-L28)。Nix 2.35.2 源码也把 `nix build` 的 JSON 接口称为“still-experimental but widely-used”：[`build.cc` 第 11–20 行](https://github.com/NixOS/nix/blob/2c73b59da29606068c0c98db015dd3a66955525d/src/nix/build.cc#L11-L20)。
- `internal-json` 记录中没有协议版本字段。其 action、activity type 和 result type 直接由内部枚举及 logger 实现编码：[`logging.hh` 第 16–43 行](https://github.com/NixOS/nix/blob/2c73b59da29606068c0c98db015dd3a66955525d/src/libutil/include/nix/util/logging.hh#L16-L43)、[`logging.cc` 第 284–355 行](https://github.com/NixOS/nix/blob/2c73b59da29606068c0c98db015dd3a66955525d/src/libutil/logging.cc#L284-L355)。官方源码和手册在这些位置没有定义独立的版本协商或兼容性承诺。

因此，Nix 2.35.2 的 `internal-json` 是官方提供给程序消费的内部接口，但不是一个带版本协商的稳定协议。消费者需要固定已支持的 Nix 版本，并容忍未知 action、type 和额外字段。

## stderr/stdout 行协议

- `--log-format internal-json` 创建写入标准错误的 JSON logger：[`loggers.cc` 第 24–33 行](https://github.com/NixOS/nix/blob/2c73b59da29606068c0c98db015dd3a66955525d/src/libmain/loggers.cc#L24-L33)。命令结果仍通过标准输出写出：[`logging.cc` 第 48–53 行](https://github.com/NixOS/nix/blob/2c73b59da29606068c0c98db015dd3a66955525d/src/libutil/logging.cc#L48-L53)。
- 每条 stderr 协议记录是 `@nix `、一段紧凑 JSON、换行符：[`logging.cc` 第 263–273 行](https://github.com/NixOS/nix/blob/2c73b59da29606068c0c98db015dd3a66955525d/src/libutil/logging.cc#L263-L273)。Nix 自己的解析器也只把以 `@nix ` 开头的行视为 JSON 日志：[`logging.cc` 第 425–434 行](https://github.com/NixOS/nix/blob/2c73b59da29606068c0c98db015dd3a66955525d/src/libutil/logging.cc#L425-L434)。
- 基本 action 为：
  - `start`：`id`、`level`、`type`、`text`、`parent`，以及可选的 positional `fields`；
  - `stop`：`id`；
  - `result`：`id`、`type`，以及可选的 positional `fields`；
  - `msg`：`level`、`msg`；错误消息还可能有 `raw_msg`、源码位置和 trace。

  这些字段由 [`logging.cc` 第 284–355 行](https://github.com/NixOS/nix/blob/2c73b59da29606068c0c98db015dd3a66955525d/src/libutil/logging.cc#L284-L355) 写出；空 `fields` 会被省略：[`logging.cc` 第 242–254 行](https://github.com/NixOS/nix/blob/2c73b59da29606068c0c98db015dd3a66955525d/src/libutil/logging.cc#L242-L254)。

解析器应逐行读取 stderr，只解析 `@nix ` 前缀后的 JSON；stdout 应独立保留给 `--print-out-paths`、`--json` 等命令结果。

## 与 Source、drvPath 和 store path 的关联

- build activity 的 type 是 `105`（`actBuild`）。它的 positional fields 是 derivation store path、构建机器名、`1`、`1`：[`derivation-building-goal.cc` 第 697–713 行](https://github.com/NixOS/nix/blob/2c73b59da29606068c0c98db015dd3a66955525d/src/libstore/build/derivation-building-goal.cc#L697-L713)。因此消费者可以从 `fields[0]` 取得 `.drv` path。
- substitute activity 的 type 是 `108`（`actSubstitute`）。它的 fields 是目标 store path 和 substituter URI：[`substitution-goal.cc` 第 230–236 行](https://github.com/NixOS/nix/blob/2c73b59da29606068c0c98db015dd3a66955525d/src/libstore/build/substitution-goal.cc#L230-L236)。
- copy-path activity 的 type 是 `100`（`actCopyPath`）。它的 fields 是 store path、源 store URI、目标 store URI：[`store-api.cc` 第 903–920 行](https://github.com/NixOS/nix/blob/2c73b59da29606068c0c98db015dd3a66955525d/src/libstore/store-api.cc#L903-L920)。
- file-transfer activity 的 type 是 `101`（`actFileTransfer`）。它的 `fields[0]` 只有请求 URL；`parent` 取自请求创建时的当前 activity：[`filetransfer.cc` 第 467–484 行](https://github.com/NixOS/nix/blob/2c73b59da29606068c0c98db015dd3a66955525d/src/libstore/filetransfer.cc#L467-L484)、[`filetransfer.hh` 第 247–254、317–320 行](https://github.com/NixOS/nix/blob/2c73b59da29606068c0c98db015dd3a66955525d/src/libstore/include/nix/store/filetransfer.hh#L247-L254)。

`internal-json` 不包含 nix-pins 的 Source 名称。若 nix-pins 为每个 Source 启动独立的 `nix build` 子进程，则子进程本身就是无歧义的 Source 归属边界；该进程的全部 activity 可以直接放入对应 Source 分组。若多个 Source 合并到一个 `nix build`，则需要在调用前保存 Source 到 drvPath/store path 的映射，再使用上述 fields 和 `parent` activity 链做关联；协议本身不会提供 Source 标识。

## 下载和复制进度

- 通用 progress result 的 type 是 `105`（`resProgress`），fields 顺序是 `done`、`expected`、`running`、`failed`：[`logging.hh` 第 219–227 行](https://github.com/NixOS/nix/blob/2c73b59da29606068c0c98db015dd3a66955525d/src/libutil/include/nix/util/logging.hh#L219-L227)。
- file-transfer activity 直接把 libcurl 回调的 `dlnow` 和 `dltotal` 写成 `done` 和 `expected`：[`filetransfer.cc` 第 487–505 行](https://github.com/NixOS/nix/blob/2c73b59da29606068c0c98db015dd3a66955525d/src/libstore/filetransfer.cc#L487-L505)。源码未保证 `dltotal` 非零；因此 `expected == 0` 时只能显示已下载字节或不定进度，不能计算百分比或 ETA。
- copy-path activity 可报告已复制的 NAR 字节和 `narSize` 总量：[`store-api.cc` 第 1064–1085 行](https://github.com/NixOS/nix/blob/2c73b59da29606068c0c98db015dd3a66955525d/src/libstore/store-api.cc#L1064-L1085)。这是 NAR 内容大小，不一定等于网络上传输的压缩字节数。
- worker 还会发送 aggregate `resSetExpected`（type `106`）：file-transfer 的 expected 来自 narinfo `fileSize`，copy-path 的 expected 来自 `narSize`：[`substitution-goal.cc` 第 101–111 行](https://github.com/NixOS/nix/blob/2c73b59da29606068c0c98db015dd3a66955525d/src/libstore/build/substitution-goal.cc#L101-L111)、[`worker.hh` 第 382–389 行](https://github.com/NixOS/nix/blob/2c73b59da29606068c0c98db015dd3a66955525d/src/libstore/include/nix/store/build/worker.hh#L382-L389)。该 expected 是当前 worker 的汇总值，不是某个声明 Source 的专属总量。

## 当前 step / phase

- result type `104`（`resSetPhase`）携带一个 phase 字符串：[`logging.hh` 第 33–43 行](https://github.com/NixOS/nix/blob/2c73b59da29606068c0c98db015dd3a66955525d/src/libutil/include/nix/util/logging.hh#L33-L43)。
- phase 来自 builder 写出的 `@nix {"action":"setPhase","phase":"..."}` 消息；Nix 将它转成当前 build activity 的 `resSetPhase`：[`logging.cc` 第 437–469 行](https://github.com/NixOS/nix/blob/2c73b59da29606068c0c98db015dd3a66955525d/src/libutil/logging.cc#L437-L469)。Nix 源码明确说明 nixpkgs stdenv 使用这种日志行上报 phase：[`derivation-building-goal.cc` 第 746–759 行](https://github.com/NixOS/nix/blob/2c73b59da29606068c0c98db015dd3a66955525d/src/libstore/build/derivation-building-goal.cc#L746-L759)。

phase 不是所有 derivation 都保证产生。UI 可以优先显示最近一次 `resSetPhase`；没有 phase 时，应根据 activity type 显示粗粒度 step，例如“查询缓存”“下载”“复制 store path”“构建”“完成”。

## 可操作结论

1. 对当前“一项 Source 对应一个 `nix build` 子进程”的模型，`internal-json` 足以按 Source 分组展示：当前版本、目标版本由 nix-pins 自己提供，当前 step 和字节进度由该子进程的 stderr activity 流提供。
2. 百分比必须是可选展示：仅在 `resProgress.fields[1] > 0` 时计算；否则显示不定进度和已完成字节。
3. 优先用子进程归属 Source，不要仅靠 URL 或日志文本反推 Source。drvPath/store path 和 parent 链可作为一个进程构建多个目标时的补充关联键。
4. 解析层应按 Nix 版本隔离，并忽略未知 action/type/字段；`internal-json` 没有协议版本字段，不能假定跨 Nix 版本完全兼容。
