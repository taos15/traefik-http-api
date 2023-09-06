import { Request, Response } from "express";
import { prisma } from "../db/client";
import { gertContainersList } from "../utils/docker/gertContainersList";

export const servers_get = (req: Request, res: Response) => {
    (async () => {
        try {
            let servers = await prisma.containerserver.findMany();
            await prisma.$disconnect();
            res.status(200).json(servers);
        } catch (err) {
            await prisma.$disconnect();
            console.log(`Endpoint requested: ${req.originalUrl}`, (err as Error).message);
            res.status(404).send("Something went wrong.");
        }
    })();
};

export const server_post = (req: Request, res: Response) => {
    (async () => {
        try {
            const { id, name, host, port, enable } = req.body;
            let servers = await prisma.containerserver.create({
                data: {
                    id,
                    name,
                    host,
                    port,
                    enable,
                },
            });
            await prisma.$disconnect();
            gertContainersList();
            res.status(201).json(servers);
        } catch (err) {
            await prisma.$disconnect();
            console.log(`Endpoint requested: ${req.originalUrl}`, (err as Error).message);
            res.status(404).send("Something went wrong.");
        }
    })();
};

export const server_put = (req: Request, res: Response) => {
    (async () => {
        try {
            const { id } = req.params;
            const { name, host, port, enable } = req.body;
            let servers = await prisma.containerserver.update({
                where: {
                    id,
                },
                data: {
                    name,
                    host,
                    port,
                    enable,
                },
            });
            await prisma.$disconnect();
            gertContainersList();
            res.status(201).json(servers);
        } catch (err) {
            await prisma.$disconnect();
            console.log(`Endpoint requested: ${req.originalUrl}`, (err as Error).message);
            res.status(400).send("Something went wrong.");
        }
    })();
};

export const server_delete = (req: Request, res: Response) => {
    (async () => {
        try {
            const id = req.params.id;
            const deletedServer = await prisma.containerserver.delete({
                where: {
                    id,
                },
            });
            await prisma.$disconnect();
            gertContainersList();
            res.status(200).json(`Server ${deletedServer.name} has been deleted`);
        } catch (err) {
            await prisma.$disconnect();
            console.log(`Endpoint requested: ${req.originalUrl}`, (err as Error).message);
            res.status(404).send("Something went wrong.");
        }
    })();
};
