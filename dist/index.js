"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
const client_1 = require("@prisma/client");
const dotenv_1 = __importDefault(require("dotenv"));
const app_1 = require("./app");
dotenv_1.default.config();
const port = 4000;
const prisma = new client_1.PrismaClient();
// async function main() {
//     // ... you will write your Prisma Client queries here
//     // const unraid = await prisma.containerserver.create({
//     //     data: {
//     //         host: "http://192.168.1.2",
//     //         name: "Unraid",
//     //         port: 2375,
//     //         enable: true,
//     //     },
//     // });
// }
// main()
//     .finally(async () => {
//         await prisma.$disconnect();
//     })
//     .catch(async (e) => {
//         console.error(e);
//         await prisma.$disconnect();
//         process.exit(1);
//     });
app_1.app.listen(port, () => {
    console.log(`[server]: Server is running at http://localhost:${port}`);
});
//# sourceMappingURL=index.js.map