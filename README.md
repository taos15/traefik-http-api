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

## API

### /api/:ver/servers/
GET/Post of the servers in the database

### /api/:ver/servers/:id
PUT modify Specify server in the database

### /api/:ver/servers/:id
DEL Delete Specify server from the database

### /api/:ver/traefikconfig
GET Traefik dynamic config in json format

## Treafik setup
you need to add a `http` provider to your traefik static config. It looks like this:
```yaml
    # http API provider
    http:
        endpoint: "http://192.168.1.1:4000/api/1/traefikconfig" # Uncomment if you are using docker socket proxy
```
Change the IP address to the IP of the server running the api server.

## Notes
The server that you the api to get the runnig containers running needs to have a dockersocket proxy running. 