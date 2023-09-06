import { User } from "@prisma/client";
import { Request, Response } from "express";
import { prisma } from "../db/client";

export const signup_post = (req: Request, res: Response) => {
    const user: User = req.body;

    (async () => {
        try {
            const newUser = await prisma.user.create({
                data: {
                    username: user.username,
                    email: user.email,
                    password: user.password,
                    admin: user.admin,
                },
            });
            await prisma.$disconnect();
            res.status(201).json(newUser);
        } catch (err) {
            await prisma.$disconnect();
            console.error(
                `Endpoint requested: ${req.originalUrl}, Failed to create user ${user.username}`,
                (err as Error).message,
            );
            res.status(400).send("User not created");
        }
    })();
};

export const login_post = (req: Request, res: Response) => {
    const user: User = req.body;

    (async () => {
        try {
            const userFound = await prisma.user.findUnique({
                where: {
                    id: user.id,
                },
            });
            await prisma.$disconnect();
            res.status(201).json(`Loged in as: ${userFound}`);
        } catch (err) {
            await prisma.$disconnect();
            console.log(`Endpoint requested: ${req.originalUrl}, Failed to login:`, (err as Error).message);
            res.status(400).send(`Login Failed`);
        }
    })();
};
export const login_get = (req: Request, res: Response) => {
    res.status(200).send("Loging in request");
};
