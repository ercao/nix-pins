// 经 workspace 子包间接使用外部依赖，同时验证子包链接和离线依赖注入。
const fs = require("node:fs");
const isNumber = require("@nix-pins/shared");

fs.writeFileSync("result.txt", `workspace:${isNumber(42)}:${isNumber("pnpm")}`);
