import { prisma } from "./app";
import { createDockerServcersInstances } from "./createDockerServcersInstances";

export async function gertContainersList() {
    const servers = await prisma.containerserver.findMany();
    const dockerServersInstances = createDockerServcersInstances(servers);
    const containerList = await Promise.all(
        dockerServersInstances.map(async (dockerInstance) => {
            const containers = await dockerInstance.listContainers({ all: true });
            return containers.map((container) => ({
                ...container,
                serverName: (dockerInstance as any).modem.headers?.name as string,
                serverHostname: (dockerInstance.modem as any).host as string,
            }));
        }),
    );
    return containerList;
}
