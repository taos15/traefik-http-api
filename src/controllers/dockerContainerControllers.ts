import Docker from "dockerode";
import { Request, Response } from "express";
import { prisma } from "../db/client";
import { findDockerInstance } from "../utils/docker/findDockerInstance";
import { gertContainersList } from "../utils/docker/gertContainersList";
import { getCombinedContainerlist } from "../utils/docker/getCombinedContainerlist";

export const dockerContainer_getAll = (req: Request, res: Response) => {
    (async () => {
        try {
            const allContainers = await getCombinedContainerlist();
            res.status(200).json(allContainers);
        } catch (err) {
            await prisma.$disconnect();
            console.log(`Endpoint requested: ${req.originalUrl}`, (err as Error).message);
            res.status(404).send("Something went wrong.");
        }
    })();
};

export const dockerContainer_getByServer = (req: Request, res: Response) => {
    (async () => {
        try {
            const { name } = req.params;
            console.log(name);
            const dockerServer = await findDockerInstance(name);

            const containers = await dockerServer.listContainers({ all: true });
            res.status(200).json(containers);
        } catch (err) {
            await prisma.$disconnect();
            console.log(`Endpoint requested: ${req.originalUrl}`, (err as Error).message);
            res.status(404).send("Something went wrong.");
        }
    })();
};
export const dockerContainer_getByID = (req: Request, res: Response) => {
    (async () => {
        try {
            const { id } = req.params;
            const { name } = req.body;
            console.log(name);
            const dockerServer = await findDockerInstance(name);

            const container = dockerServer.getContainer(id);

            res.status(200).json(await container.inspect());
        } catch (err) {
            await prisma.$disconnect();
            console.log(`Endpoint requested: ${req.originalUrl}`, (err as Error).message);
            res.status(404).send("Something went wrong.");
        }
    })();
};
export const dockerContainer_postByID = (req: Request, res: Response) => {
    (async () => {
        try {
            const { id } = req.params;
            const { name, command }: { name: string; command: string } = req.body;
            console.log(name);
            const dockerServer = await findDockerInstance(name);

            const container = dockerServer.getContainer(id);

            switch (command) {
                case "start":
                    await container.start();
                    break;
                case "stop":
                    await container.stop();
                    break;
                case "pause":
                    await container.pause();
                    break;
                case "unpause":
                    await container.unpause();
                    break;
                case "restart":
                    await container.restart();
                    break;
                default:
                    throw new Error("Command:  " + command + ", is invalid.");
            }

            res.status(200).json(`${command} command have been sent to the container`);
        } catch (err) {
            await prisma.$disconnect();
            console.log(`Endpoint requested: ${req.originalUrl}`, (err as Error).message);
            res.status(404).send("Something went wrong.");
        }
    })();
};

export const dockerContainer_post = (req: Request, res: Response) => {
    (async () => {
        try {
            res.status(200).json("Post response");
        } catch (err) {
            await prisma.$disconnect();
            console.log(`Endpoint requested: ${req.originalUrl}`, (err as Error).message);
            res.status(404).send("Something went wrong.");
        }
    })();
};

export const dockerContainer_put = (req: Request, res: Response) => {
    (async () => {
        try {
            res.status(200).json("Put response");
        } catch (err) {
            await prisma.$disconnect();
            console.log(`Endpoint requested: ${req.originalUrl}`, (err as Error).message);
            res.status(400).send("Something went wrong.");
        }
    })();
};

export const dockerContainer_delete = (req: Request, res: Response) => {
    (async () => {
        try {
            res.status(200).json("Delete response");
        } catch (err) {
            await prisma.$disconnect();
            console.log(`Endpoint requested: ${req.originalUrl}`, (err as Error).message);
            res.status(404).send("Something went wrong.");
        }
    })();
};
