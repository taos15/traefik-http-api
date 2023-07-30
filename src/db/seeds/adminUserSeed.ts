import { prisma } from "../client";

export default async function adminUserSeed() {
    const defaultAdmin = await prisma.user.upsert({
        where: { username: "admin" },
        update: {},
        create: {
            username: "admin",
            email: "admin@yourEmail.com",
            password: "admin",
            admin: true,
        },
    });
    return defaultAdmin;
}
