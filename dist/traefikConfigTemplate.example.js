"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.traefik = void 0;
exports.traefik = {
    http: {
        routers: {},
        services: {},
        middlewares: {
            ["local-ipwhitelist"]: { ipWhiteList: { sourceRange: ["127.0.0.1/32", "192.168.1.1/24"] } },
            auth: {
                forwardauth: {
                    // Change the ip: port to match your authelia, and domain.tld to match you domain name
                    address: "http://192.168.1.1:9091/api/verify?rd=https://authelia.yourdomain.tld/",
                    trustForwardHeader: true,
                    authResponseHeaders: ["Remote-User", "Remote-Groups", "Remote-Name", "Remote-Email"],
                },
            },
            ["auth-basic"]: {
                forwardauth: {
                    // Change the ip: port to match your authelia
                    address: "http://192.168.1.1:9091/api/verify?authelia=basic",
                    trustForwardHeader: true,
                    authResponseHeaders: ["Remote-User", "Remote-Groups", "Remote-Name", "Remote-Email"],
                },
            },
            securityHeaders: {
                headers: {
                    customResponseHeaders: {
                        ["X-Robots-Tag"]: "none,noarchive,nosnippet,notranslate,noimageindex",
                        server: "",
                        ["X-Forwarded-Proto"]: "https",
                    },
                    sslProxyHeaders: {
                        ["X-Forwarded-Proto"]: "https",
                    },
                    referrerPolicy: "strict-origin-when-cross-origin",
                    hostsProxyHeaders: ["X-Forwarded-Host"],
                    customRequestHeaders: {
                        ["X-Forwarded-Proto"]: "https",
                    },
                    contentTypeNosniff: true,
                    browserXssFilter: true,
                    forceSTSHeader: true,
                    stsIncludeSubdomains: true,
                    stsSeconds: 63072000,
                    stsPreload: true,
                },
            },
        },
    },
};
//# sourceMappingURL=traefikConfigTemplate.example.js.map