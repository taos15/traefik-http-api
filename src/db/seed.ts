import { prisma } from "./client";
import adminUserSeed from "./seeds/adminUserSeed";

async function main() {
    const defaultAdmin = adminUserSeed();
    console.log({ defaultAdmin });
}
main()
    .then(async () => {
        await prisma.$disconnect();
    })
    .catch(async (e) => {
        console.error(e);
        await prisma.$disconnect();
        process.exit(1);
    });
