import { PrismaClient } from "@prisma/client";
import cors from "cors";
import Docker from "dockerode";
import express, { Request, Response } from "express";
import morgan from "morgan";
import { traefik } from "./config/traefikConfigTemplate";

export const app = express();

const prisma = new PrismaClient();

app.use(cors());
app.use(express.json());
app.use(morgan("dev"));

let congFile;
const domain = process.env.DOMAIN;

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

app.get("/api/:ver/traefikconfig", async (req: Request, res: Response) => {
    try {
        const servers = await prisma.containerserver.findMany();
        const dockerServersInstances = servers.map((docker: any) => {
            return new Docker({
                host: docker.host,
                port: docker.port,
                headers: { name: docker.name },
            });
        });
        const containerList = await Promise.all(
            dockerServersInstances.map(async (dockerInstance: any) => {
                const containers = await dockerInstance.listContainers(req.query);
                return containers.map((container: any) => ({
                    ...container,
                    serverName: (dockerInstance as any).modem.headers?.name as string,
                    serverHostname: (dockerInstance.modem as any).host as string,
                }));
            }),
        );
        const mergedContainerList = [...containerList.flat()];
        congFile = { http: { routers: "authelia" } };
        const routers = mergedContainerList.map((item) => {
            const route = {
                Id: item.Id,
                Name: item.Names[0],
                Image: item.Image,
                Ports: item.Ports,
                Labels: item.Labels,
                State: item.State,
                HostConfig: item.HostConfig,
                NetworkSettings: item.NetworkSettings,
                Mounts: item.Mounts,
                serverName: item.serverName,
                serverHostname: item.serverHostname,
            };
            return route;
        });
        const traefikRoutes = routers.filter(
            (itemTofilter) =>
                itemTofilter.Labels["traefik.enable"] === "true" || itemTofilter.Labels["swag"] === "enable",
        );
        const filteredRoutes = traefikRoutes.map((container) => {
            const keyName =
                container.Labels["traefik.name"] ??
                container.Name.replace(/^\//, "").replace(/^\w/, (c: any) => c.toUpperCase());
            const constainerHostname = container.Labels["traefik.hostname"] ?? keyName;
            const containerMiddlewares: string[] =
                container.Labels["traefik.middlewares"]?.split(",") ??
                (container.Labels["authelia_auth"] === "false" ? [] : ["auth"]);
            const containerEntrypoints = container.Labels["traefik.entrypoints"]?.split(",") ?? ["https"];
            return {
                [keyName]: {
                    entryPoints: containerEntrypoints,
                    rule: `Host(` + `\`` + constainerHostname + `.${domain}\`)`,
                    service: keyName,
                    middlewares: containerMiddlewares,
                },
            };
        });

        Object.assign(traefik.http.routers, ...filteredRoutes);

        const filteredServices = traefikRoutes.map((container) => {
            const keyName = container.Name.replace(/^\//, "").replace(/^\w/, (c: any) => c.toUpperCase());
            const containerWebuiPort =
                container.Labels["traefik.webuiport"]?.split(",") ??
                container.Ports[container.Ports.findIndex((obj: any) => obj.IP === "0.0.0.0")]?.PublicPort;
            return {
                [keyName]: {
                    loadBalancer: {
                        servers: [
                            {
                                url: `${container.serverHostname}:${containerWebuiPort}`,
                            },
                        ],
                    },
                },
            };
        });

        Object.assign(traefik.http.services, ...filteredServices);
        await prisma.$disconnect();
        res.status(200).send(JSON.stringify(traefik));
    } catch (err) {
        await prisma.$disconnect();
        console.log(`Endpoint requested: ${req.originalUrl}`, err);
        res.status(500).send("Something went wrong.");
    }
});
