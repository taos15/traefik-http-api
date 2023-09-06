import { Router } from "express";
import { login_get, login_post, signup_post } from "../controllers/authContollers";

export const authRouter = Router();
authRouter.post("/signup", signup_post);
authRouter.get("/login", login_get);
authRouter.post("/login", login_post);
