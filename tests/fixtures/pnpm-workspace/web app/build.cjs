const fs = require("node:fs");
const isNumber = require("@nix-pins/shared");

fs.writeFileSync("result.txt", `workspace:${isNumber(42)}:${isNumber("pnpm")}`);
