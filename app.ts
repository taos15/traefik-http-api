import { PrismaClient } from "@prisma/client";
import cors from "cors";
import Docker from "dockerode";
import express from "express";
import morgan from "morgan";
import path from "path";
import { traefik } from "./config/traefikConfigTemplate";

export const app = express();

const prisma = new PrismaClient();

app.use(cors());
app.use(express.json());
app.use(morgan("dev"));
app.use(express.static(path.join(__dirname, "./ui")));

let congFile;

app.get("/api/:ver/servers", async (req, res) => {
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

app.post("/api/:ver/servers/", async (req, res) => {
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
app.put("/api/:ver/servers/:id", async (req, res) => {
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

app.delete("/api/:ver/servers/:id", async (req, res) => {
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

app.get("/api/:ver/", (req, res) => {
    const version = req.params.ver;
    res.status(200).send(`server up and running, api version: ${version}`);
});

app.get("/api/:ver/traefikconfig", async (req, res) => {
    try {
        const servers = await prisma.containerserver.findMany();
        const dockerServersInstances = servers.map((docker) => {
            return new Docker({
                host: docker.host,
                port: docker.port,
                headers: { name: docker.name },
            });
        });

        const containerList = await Promise.all(
            dockerServersInstances.map(async (dockerInstance) => {
                const containers = await dockerInstance.listContainers(req.query);
                return containers.map((container) => ({
                    ...container,
                    serverName: (dockerInstance as any).modem.headers?.name,
                    serverHostname: (dockerInstance.modem as any).host,
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
        // const traefikRoutes = routers.filter((itemTofilter) => itemTofilter.Labels["traefik.enable"] === "true");
        const filteredRoutes = traefikRoutes.map((container) => {
            const keyName = container.Name.replace(/^\//, "").replace(/^\w/, (c) => c.toUpperCase());
            return {
                [keyName]: {
                    entryPoints: ["https"],
                    rule: `Host(` + `\`` + keyName + `.taos15.net\`)`,
                    service: keyName,
                    middlewares: ["auth"],
                },
            };
        });

        Object.assign(traefik.http.routers, ...filteredRoutes);

        const filteredServices = traefikRoutes.map((container) => {
            const keyName = container.Name.replace(/^\//, "").replace(/^\w/, (c) => c.toUpperCase());
            return {
                [keyName]: {
                    loadBalancer: {
                        servers: [
                            {
                                url: `${container.serverHostname}:${
                                    container.Ports[container.Ports.findIndex((obj) => obj.IP === "0.0.0.0")]
                                        ?.PublicPort
                                }`,
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
