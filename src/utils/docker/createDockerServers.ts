import { prisma } from "../../db/client";
import { createDockerServersInstances } from "./createDockerServcersInstances";

export async function createDockerServers() {
    let servers = await prisma.containerserver.findMany();
    let dockerServersInstances = createDockerServersInstances(servers);
    return dockerServersInstances;
}
