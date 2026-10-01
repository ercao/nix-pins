const fs = require("node:fs");
const isNumber = require("is-number");

fs.writeFileSync("result.txt", `${isNumber(42)}:${isNumber("pnpm")}`);
