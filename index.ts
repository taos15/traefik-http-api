import { PrismaClient } from "@prisma/client";
import dotenv from "dotenv";
import { app } from "./app";

dotenv.config();

const port = 4000;

const prisma = new PrismaClient();

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

app.listen(port, () => {
    console.log(`[server]: Server is running at http://localhost:${port}`);
});
