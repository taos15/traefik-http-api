import Docker from "dockerode";
import { Icontainerserver } from "./app";

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
