// 用真实依赖计算可断言的产物，验证构建阶段能够消费预先锁定的 pnpm 依赖。
const fs = require("node:fs");
const isNumber = require("is-number");

fs.writeFileSync("result.txt", `${isNumber(42)}:${isNumber("pnpm")}`);
