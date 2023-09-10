import { prisma } from "../../db/client";
import { createDockerServers } from "./createDockerServers";

export async function gertContainersList() {
    try {
        let dockerServersInstances = await createDockerServers();
        let containerList = await Promise.all(
            dockerServersInstances.map(async (dockerInstance) => {
                let containers = await dockerInstance.listContainers({ all: true });
                return containers.map((container) => ({
                    ...container,
                    serverName: (dockerInstance as any).modem.headers?.name as string,
                    serverHostname: (dockerInstance.modem as any).host as string,
                }));
            }),
        );
        await prisma.$disconnect();
        return containerList;
    } catch (error) {
        console.error("there was an error while getting the containers from the database:", error);
    }
}
