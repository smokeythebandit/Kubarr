FROM alpine@sha256:4bcff63911fcb4448bd4fdacec207030997caf25e9bea4045fa6c8c44de311d1

RUN apk add --no-cache curl iproute2 iptables socat wireguard-tools

COPY endpoint-request.sh endpoint.sh server.sh /usr/local/bin/
RUN chmod 0755 /usr/local/bin/endpoint-request.sh /usr/local/bin/endpoint.sh /usr/local/bin/server.sh

CMD ["/usr/local/bin/server.sh"]
