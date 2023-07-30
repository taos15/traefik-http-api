import cors from "cors";
import express, { Request, Response } from "express";
import morgan from "morgan";
import { addRoutesToTraefik } from "./addRoutesToTraefik";
import { addServicesToTraefik } from "./addServicesToTraefik";
import { traefik } from "./config/traefikConfigTemplate";
import { prisma } from "./db/client";
import { getTraefikContainers } from "./getTraefikContainers";
import { authRouter } from "./routes/authRoute";

export const app = express();

app.use(cors());
app.use(express.json());
app.use(morgan("dev"));
app.use(authRouter);

export const domain = process.env.DOMAIN;

export interface Icontainerserver {
    id: string;
    name: string;
    host: string;
    port: number;
    enable: boolean;
}
app.get("/api/:ver/test", (req: Request, res: Response) => {
    (async () => {
        const mergedContainerList = await getTraefikContainers();
        res.status(200).send(mergedContainerList);
    })();
});

app.get("/api/:ver/servers", (req: Request, res: Response) => {
    (async () => {
        try {
            const servers = await prisma.containerserver.findMany();
            await prisma.$disconnect();
            res.status(200).json(servers);
        } catch (err) {
            await prisma.$disconnect();
            console.log(`Endpoint requested: ${req.originalUrl}`, err);
            res.status(400).send("Something went wrong.");
        }
    })();
});

app.post("/api/:ver/servers/", (req: Request, res: Response) => {
    (async () => {
        try {
            const { id, name, host, port, enable } = req.body;
            const servers = await prisma.containerserver.create({
                data: {
                    id,
                    name,
                    host,
                    port,
                    enable,
                },
            });
            await prisma.$disconnect();
            res.status(201).json(servers);
        } catch (err) {
            await prisma.$disconnect();
            console.log(`Endpoint requested: ${req.originalUrl}`, err);
            res.status(400).send("Something went wrong.");
        }
    })();
});
app.put("/api/:ver/servers/:id", (req: Request, res: Response) => {
    (async () => {
        try {
            const id = req.params.id;
            const { name, host, port, enable } = req.body;
            const servers = await prisma.containerserver.update({
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
            res.status(201).json(servers);
        } catch (err) {
            await prisma.$disconnect();
            console.log(`Endpoint requested: ${req.originalUrl}`, err);
            res.status(400).send("Something went wrong.");
        }
    })();
});

app.delete("/api/:ver/servers/:id", (req: Request, res: Response) => {
    (async () => {
        try {
            const id = req.params.id;
            const deletedServer = await prisma.containerserver.delete({
                where: {
                    id,
                },
            });
            await prisma.$disconnect();
            res.status(200).json(`Server ${deletedServer.name} has been deleted`);
        } catch (err) {
            await prisma.$disconnect();
            console.log(`Endpoint requested: ${req.originalUrl}`, err);
            res.status(400).send("Something went wrong.");
        }
    })();
});

app.get("/api/:ver/", (req: Request, res: Response) => {
    const version = req.params.ver;
    res.status(200).send(`server up and running, api version: ${version}`);
});

app.get("/api/:ver/traefikconfig", (req: Request, res: Response) => {
    (async () => {
        try {
            addRoutesToTraefik();
            addServicesToTraefik();

            await prisma.$disconnect();
            res.status(200).send(JSON.stringify(traefik));
        } catch (err) {
            await prisma.$disconnect();
            console.log(`Endpoint requested: ${req.originalUrl}`, err);
            res.status(400).send("Something went wrong.");
        }
    })();
});

app.get("*", (req: Request, res: Response) => {
    (async () => {
        try {
            res.status(200).json("Server RUNNING");
        } catch (err) {
            await prisma.$disconnect();
            console.log(`Endpoint requested: ${req.originalUrl}`, err);
            res.status(400).send("Something went wrong.");
        }
    })();
});
