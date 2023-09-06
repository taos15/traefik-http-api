import { Request, Response } from "express";
import { traefik } from "../config/traefikConfigTemplate";
import { prisma } from "../db/client";
import { addRoutesToTraefik } from "../utils/traefik/addRoutesToTraefik";
import { addServicesToTraefik } from "../utils/traefik/addServicesToTraefik";

export const traefikConfig_get = (req: Request, res: Response) => {
    (async () => {
        try {
            traefik.http.routers = {};
            traefik.http.services = {};
            await addRoutesToTraefik();
            await addServicesToTraefik();

            await prisma.$disconnect();
            res.status(200).json(traefik);
        } catch (err) {
            await prisma.$disconnect();
            console.log(`Endpoint requested: ${req.originalUrl}`, (err as Error).message);
            res.status(404).send("Something went wrong.");
        }
    })();
};
