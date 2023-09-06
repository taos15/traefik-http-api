import Docker from "dockerode";
import { Icontainerserver } from "../../app";
let dockerServersInstancesHolder: Docker[] = [];
export const createDockerServersInstances = (servers: Icontainerserver[]): Docker[] => {
    if (dockerServersInstancesHolder.length !== servers.length) {
        let dockerServersInstances = servers.map((docker) => {
            return new Docker({
                host: docker.host,
                port: docker.port,
                headers: { name: docker.name },
            });
        });
        dockerServersInstancesHolder = dockerServersInstances;
    }

    return dockerServersInstancesHolder;
};
