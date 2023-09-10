import { createDockerServers } from "./createDockerServers";

export async function findDockerInstance(name: string) {
    let dockerServersInstances = await createDockerServers();

    const indexOfServer = dockerServersInstances.findIndex((server: any) => server.modem.headers?.name === name);
    return dockerServersInstances[indexOfServer];
}
