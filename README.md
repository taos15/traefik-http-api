# traefik-http-api
work in process


# Instalation
1. Clone repo
1. Run `pnpm i`
1. Copy the `.env.example` file and rename it to `.env`
1. Rename the `traefikConfigTemplate.ts.example` to `traefikConfigTemplate.ts` and change the authelia ip and you domain

## Run the App

To run the app run:
```bash
pnpm run build
pnpm run start

```
### The app will be available at you localhost port 4001
- If you get an error saying that the port is been use you can run `lsof -ti :4001 | xargs kill -9` to kill the process running in that port.