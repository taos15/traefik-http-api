import { PrismaClient } from "@prisma/client";
import cors from "cors";
import Docker from "dockerode";
import express, { Request, Response } from "express";
import morgan from "morgan";
import { traefik } from "./config/traefikConfigTemplate";
import { gertContainersList } from "./gertContainersList";
import { getAllContainers } from "./getAllContainers";
import { getCombinedContainerlist } from "./getCombinedContainerlist";
import { getFilteredRoutes } from "./getFilteredRoutes";
import { getFilteredServices } from "./getFilteredServices";
import { getTraefikContainers } from "./getTraefikContainers";

export const app = express();

export const prisma = new PrismaClient();

app.use(cors());
app.use(express.json());
app.use(morgan("dev"));

let congFile;
export const domain = process.env.DOMAIN;

export interface Icontainerserver {
    id: string;
    name: string;
    host: string;
    port: number;
    enable: boolean;
}

export const createDockerServcersInstances = (servers: Icontainerserver[]): Docker[] => {
    const dockerServersInstances = servers.map((docker) => {
        return new Docker({
            host: docker.host,
            port: docker.port,
            headers: { name: docker.name },
        });
    });
    return dockerServersInstances;
};
app.get("/api/:ver/servers", async (req: Request, res: Response) => {
    try {
        const servers = await prisma.containerserver.findMany();
        await prisma.$disconnect();
        res.status(200).json(servers);
    } catch (err) {
        await prisma.$disconnect();
        console.log(`Endpoint requested: ${req.originalUrl}`, err);
        res.status(500).send("Something went wrong.");
    }
});

app.post("/api/:ver/servers/", async (req: Request, res: Response) => {
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
        res.status(500).send("Something went wrong.");
    }
});
app.put("/api/:ver/servers/:id", async (req: Request, res: Response) => {
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
        res.status(500).send("Something went wrong.");
    }
});

app.delete("/api/:ver/servers/:id", async (req: Request, res: Response) => {
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
        res.status(500).send("Something went wrong.");
    }
});

app.get("/api/:ver/", (req: Request, res: Response) => {
    const version = req.params.ver;
    res.status(200).send(`server up and running, api version: ${version}`);
});
app.get("/api/:ver/test", async (req: Request, res: Response) => {
    const mergedContainerList = await getTraefikContainers();
    res.status(200).send(mergedContainerList);
});

app.get("/api/:ver/traefikconfig", async (req: Request, res: Response) => {
    try {
        congFile = { http: { routers: "authelia" } };
        const filteredRoutes = await getFilteredRoutes();

        addRoutesToTraefik(filteredRoutes);

        const filteredServices = await getFilteredServices();

        addServicesToTraefik(filteredServices);
        await prisma.$disconnect();
        res.status(200).send(JSON.stringify(traefik));
    } catch (err) {
        await prisma.$disconnect();
        console.log(`Endpoint requested: ${req.originalUrl}`, err);
        res.status(500).send("Something went wrong.");
    }
});

app.get("*", async (req: Request, res: Response) => {
    try {
        res.status(200).json("Server RUNNING");
    } catch (err) {
        await prisma.$disconnect();
        console.log(`Endpoint requested: ${req.originalUrl}`, err);
        res.status(500).send("Something went wrong.");
    }
});
function addServicesToTraefik(filteredServices: { [x: string]: { loadBalancer: { servers: { url: string }[] } } }[]) {
    Object.assign(traefik.http.services, ...filteredServices);
}

function addRoutesToTraefik(
    filteredRoutes: { [x: string]: { entryPoints: string[]; rule: string; service: string; middlewares: string[] } }[],
) {
    Object.assign(traefik.http.routers, ...filteredRoutes);
}
