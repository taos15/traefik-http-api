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
    const containerList = await gertContainersList();
    const mergedContainerList = [...containerList.flat()];
    res.status(200).send(mergedContainerList);
});

app.get("/api/:ver/traefikconfig", async (req: Request, res: Response) => {
    try {
        const containerList = await gertContainersList();
        const mergedContainerList = [...containerList.flat()];
        congFile = { http: { routers: "authelia" } };
        const containers = getAllContainers(mergedContainerList);
        const traefikContainers = containers.filter(
            (itemTofilter) =>
                itemTofilter.Labels["traefik.enable"] === "true" || itemTofilter.Labels["swag"] === "enable",
        );
        const filteredRoutes = getFilteredRoutes(traefikContainers);

        Object.assign(traefik.http.routers, ...filteredRoutes);

        const filteredServices = getFilteredServices(traefikContainers);

        Object.assign(traefik.http.services, ...filteredServices);
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
function getFilteredServices(
    traefikContainers: {
        Id: string;
        Name: string;
        Image: string;
        Ports: Docker.Port[];
        Labels: { [label: string]: string };
        State: string;
        HostConfig: { NetworkMode: string };
        NetworkSettings: { Networks: { [networkType: string]: Docker.NetworkInfo } };
        Mounts: {
            Name?: string | undefined;
            Type: string;
            Source: string;
            Destination: string;
            Driver?: string | undefined;
            Mode: string;
            RW: boolean;
            Propagation: string;
        }[];
        serverName: string;
        serverHostname: string;
    }[],
) {
    return traefikContainers.map((container) => {
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
}

function getFilteredRoutes(
    traefikContainers: {
        Id: string;
        Name: string;
        Image: string;
        Ports: Docker.Port[];
        Labels: { [label: string]: string };
        State: string;
        HostConfig: { NetworkMode: string };
        NetworkSettings: { Networks: { [networkType: string]: Docker.NetworkInfo } };
        Mounts: {
            Name?: string | undefined;
            Type: string;
            Source: string;
            Destination: string;
            Driver?: string | undefined;
            Mode: string;
            RW: boolean;
            Propagation: string;
        }[];
        serverName: string;
        serverHostname: string;
    }[],
) {
    return traefikContainers.map((container) => {
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
}

function getAllContainers(
    mergedContainerList: {
        serverName: string;
        serverHostname: string;
        Id: string;
        Names: string[];
        Image: string;
        ImageID: string;
        Command: string;
        Created: number;
        Ports: Docker.Port[];
        Labels: { [label: string]: string };
        State: string;
        Status: string;
        HostConfig: { NetworkMode: string };
        NetworkSettings: { Networks: { [networkType: string]: Docker.NetworkInfo } };
        Mounts: {
            Name?: string | undefined;
            Type: string;
            Source: string;
            Destination: string;
            Driver?: string | undefined;
            Mode: string;
            RW: boolean;
            Propagation: string;
        }[];
    }[],
) {
    return mergedContainerList.map((item) => {
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
}

async function gertContainersList() {
    const servers = await prisma.containerserver.findMany();
    const dockerServersInstances = createDockerServcersInstances(servers);
    const containerList = await Promise.all(
        dockerServersInstances.map(async (dockerInstance) => {
            const containers = await dockerInstance.listContainers();
            return containers.map((container) => ({
                ...container,
                serverName: (dockerInstance as any).modem.headers?.name as string,
                serverHostname: (dockerInstance.modem as any).host as string,
            }));
        }),
    );
    return containerList;
}
