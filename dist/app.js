"use strict";
var __awaiter = (this && this.__awaiter) || function (thisArg, _arguments, P, generator) {
    function adopt(value) { return value instanceof P ? value : new P(function (resolve) { resolve(value); }); }
    return new (P || (P = Promise))(function (resolve, reject) {
        function fulfilled(value) { try { step(generator.next(value)); } catch (e) { reject(e); } }
        function rejected(value) { try { step(generator["throw"](value)); } catch (e) { reject(e); } }
        function step(result) { result.done ? resolve(result.value) : adopt(result.value).then(fulfilled, rejected); }
        step((generator = generator.apply(thisArg, _arguments || [])).next());
    });
};
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.app = void 0;
const client_1 = require("@prisma/client");
const cors_1 = __importDefault(require("cors"));
const dockerode_1 = __importDefault(require("dockerode"));
const express_1 = __importDefault(require("express"));
const morgan_1 = __importDefault(require("morgan"));
const path_1 = __importDefault(require("path"));
const traefikConfigTemplate_1 = require("./config/traefikConfigTemplate");
exports.app = (0, express_1.default)();
const prisma = new client_1.PrismaClient();
exports.app.use(express_1.default.json());
exports.app.use((0, morgan_1.default)("dev"));
exports.app.use((0, cors_1.default)());
exports.app.use(express_1.default.static(path_1.default.join(__dirname, "./ui")));
let congFile;
exports.app.get("/api/:ver/servers", (req, res) => __awaiter(void 0, void 0, void 0, function* () {
    try {
        const servers = yield prisma.containerserver.findMany();
        yield prisma.$disconnect();
        res.status(200).json(servers);
    }
    catch (err) {
        yield prisma.$disconnect();
        console.log(`Endpoint requested: ${req.originalUrl}`, err);
        res.status(500).send("Something went wrong.");
    }
}));
exports.app.post("/api/:ver/servers/", (req, res) => __awaiter(void 0, void 0, void 0, function* () {
    try {
        const { id, name, host, port, enable } = req.body;
        const servers = yield prisma.containerserver.create({
            data: {
                id,
                name,
                host,
                port,
                enable,
            },
        });
        yield prisma.$disconnect();
        res.status(201).json(servers);
    }
    catch (err) {
        yield prisma.$disconnect();
        console.log(`Endpoint requested: ${req.originalUrl}`, err);
        res.status(500).send("Something went wrong.");
    }
}));
exports.app.put("/api/:ver/servers/:id", (req, res) => __awaiter(void 0, void 0, void 0, function* () {
    try {
        const id = req.params.id;
        const { name, host, port, enable } = req.body;
        const servers = yield prisma.containerserver.update({
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
        yield prisma.$disconnect();
        res.status(201).json(servers);
    }
    catch (err) {
        yield prisma.$disconnect();
        console.log(`Endpoint requested: ${req.originalUrl}`, err);
        res.status(500).send("Something went wrong.");
    }
}));
exports.app.delete("/api/:ver/servers/:id", (req, res) => __awaiter(void 0, void 0, void 0, function* () {
    try {
        const id = req.params.id;
        const deletedServer = yield prisma.containerserver.delete({
            where: {
                id,
            },
        });
        yield prisma.$disconnect();
        res.status(200).json(`Server ${deletedServer.name} has been deleted`);
    }
    catch (err) {
        yield prisma.$disconnect();
        console.log(`Endpoint requested: ${req.originalUrl}`, err);
        res.status(500).send("Something went wrong.");
    }
}));
exports.app.get("/api/:ver/", (req, res) => {
    const version = req.params.ver;
    res.status(200).send(`server up and running, api version: ${version}`);
});
exports.app.get("/api/:ver/traefikconfig", (req, res) => __awaiter(void 0, void 0, void 0, function* () {
    try {
        const servers = yield prisma.containerserver.findMany();
        const dockerServersInstances = servers.map((docker) => {
            return new dockerode_1.default({
                host: docker.host,
                port: docker.port,
                headers: { name: docker.name },
            });
        });
        const containerList = yield Promise.all(dockerServersInstances.map((dockerInstance) => __awaiter(void 0, void 0, void 0, function* () {
            const containers = yield dockerInstance.listContainers(req.query);
            return containers.map((container) => {
                var _a;
                return (Object.assign(Object.assign({}, container), { serverName: (_a = dockerInstance.modem.headers) === null || _a === void 0 ? void 0 : _a.name, serverHostname: dockerInstance.modem.host }));
            });
        })));
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
        const traefikRoutes = routers.filter((itemTofilter) => itemTofilter.Labels["traefik.enable"] === "true" || itemTofilter.Labels["swag"] === "enable");
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
        Object.assign(traefikConfigTemplate_1.traefik.http.routers, ...filteredRoutes);
        const filteredServices = traefikRoutes.map((container) => {
            var _a;
            const keyName = container.Name.replace(/^\//, "").replace(/^\w/, (c) => c.toUpperCase());
            return {
                [keyName]: {
                    loadBalancer: {
                        servers: [
                            {
                                url: `${container.serverHostname}:${(_a = container.Ports[container.Ports.findIndex((obj) => obj.IP === "0.0.0.0")]) === null || _a === void 0 ? void 0 : _a.PublicPort}`,
                            },
                        ],
                    },
                },
            };
        });
        Object.assign(traefikConfigTemplate_1.traefik.http.services, ...filteredServices);
        yield prisma.$disconnect();
        res.status(200).send(JSON.stringify(traefikConfigTemplate_1.traefik));
    }
    catch (err) {
        yield prisma.$disconnect();
        console.log(`Endpoint requested: ${req.originalUrl}`, err);
        res.status(500).send("Something went wrong.");
    }
    exports.app.get("*", (req, res) => {
        res.sendFile(path_1.default.join(__dirname, "./ui/index.html"));
    });
}));
//# sourceMappingURL=app.js.map