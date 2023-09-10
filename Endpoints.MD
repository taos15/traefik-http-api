# Documentation

## Traefik-http-api

### Server

#### GET Servers

```http
localhost/api/v1/servers
```

#### GET Server By Name

```http
localhost/api/v1/servers/:name
```

#### POST Create Servers

```http
localhost/api/v1/servers
```

##### Body

```json
{
    "name": "test",
    "host": "<http://192.168.1.32>",
    "port": 2375,
    "enable": true
}
```

#### PUT Edit Server

```http
localhost/api/v1/servers/:id
```

##### Body

```json
{
    "name": "test4",
    "host": "<http://192.168.1.32>",
    "port": 2375,
    "enable": true
}
```

#### DELETE Delete Server

```http
localhost/api/v1/servers/:id
```

##### Body

```json
{
    "name": "test4",
    "host": "<http://192.168.1.32>",
    "port": 2375,
    "enable": true
}
```

### Docker Container

#### GET Containers All

```http
localhost/api/v1/containers
```

#### GET Containers By Server

```http
localhost/api/v1/containers/server/:name
```

#### GET Container By Id

```http
localhost/api/v1/containers/:id
```

##### Body

```json
{
    "name": "Servername"
}
```

#### POST Container Command

The following commands are available: start, stop, pause, unpause, and restart.

```http
localhost/api/v1/containers/:id
```

##### Body

```json
{
    "name": "ServerName",
    "command": "start"
}
```

### Traefik

#### GET Traefik Config

```http
localhost/api/v1/traefikconfig
```
