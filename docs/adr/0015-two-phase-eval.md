# 配置以两阶段求值，版本经 pins 参数注入

status: accepted

用户配置中的 `rev` 不写字面量，而是引用 `pins.<name>.version`，由工具注入。工具不改写用户的 Nix 文件（那需要保留格式的 Nix 编辑器），版本这一事实只存在于 Pins File 一处。

配置签名为 `{ pkgs, fake, pins }`。工具求值两次：

```
阶段一   pins = {}                            只读 .check
阶段二   pins = { curlie.version = "v1.8.2"; }  读 .src.drvPath 与各中间 FOD drvPath
```

Nix 的惰性使阶段一无需占位版本：`.check` 不引用 `pins`，因此 `.src` 中的 `pins.curlie.version` 不被求值。本机实测阶段一在 `pins = {}` 下正常返回：

```json
{"curlie":{"github":"rs/curlie"},"sloc":{"github":"flosse/sloc"}}
```

阶段二注入版本后返回：

```json
{"curlie":{"src":"/nix/store/845ljn1...-source.drv",
           "derived":{"vendorHash":"/nix/store/cx1wkn8...-curlie-v1.8.2-go-modules.drv"}},
 "sloc":  {"src":"/nix/store/c1a6847...-source.drv",
           "derived":{"npmDepsHash":"/nix/store/8mp1pcy...-sloc-v0.3.2-npm-deps.drv"}}}
```

## Considered Options

- 工具改写用户配置中的 `rev` 字面量：需要保留注释与格式的 Nix 编辑器，比 TOML 的 `toml_edit` 更困难，且用户手写内容会被机器改动。
- 阶段一注入占位版本：不需要。惰性求值已使其可省，且占位值会使 src 的 drvPath 算错。

## Consequences

配置约束：`.check` 不得依赖 `pins`。违反时 Nix 报 `error: attribute '<name>' missing`，指向配置中的具体位置，诊断清晰。

`status` 子命令也需一次 eval（实测 warm 约 0.49s）。
