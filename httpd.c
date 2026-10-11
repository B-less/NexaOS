#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/socket.h>
#include <netinet/in.h>

void serve_client(int client_fd) {
    char buffer[1024];
    read(client_fd, buffer, sizeof(buffer) - 1);
    
    char response[] = "HTTP/1.1 200 OK\r\n"
                      "Content-Type: text/html\r\n"
                      "Connection: close\r\n\r\n"
                      "<html><body><h1>Welcome to Nexa OS Web Server!</h1><p>Running entirely in C.</p></body></html>";
                      
    write(client_fd, response, sizeof(response) - 1);
    close(client_fd);
}

int main() {
    int server_fd;
    struct sockaddr_in address;
    int opt = 1;

    if ((server_fd = socket(AF_INET, SOCK_STREAM, 0)) == 0) {
        perror("socket failed");
        exit(EXIT_FAILURE);
    }

    setsockopt(server_fd, SOL_SOCKET, SO_REUSEADDR, &opt, sizeof(opt));

    address.sin_family = AF_INET;
    address.sin_addr.s_addr = INADDR_ANY;
    address.sin_port = htons(8000);

    if (bind(server_fd, (struct sockaddr *)&address, sizeof(address)) < 0) {
        perror("bind failed");
        exit(EXIT_FAILURE);
    }

    if (listen(server_fd, 10) < 0) {
        perror("listen");
        exit(EXIT_FAILURE);
    }

    printf("NexaOS httpd listening on port 8000...\n");

    while (1) {
        int client_fd = accept(server_fd, NULL, NULL);
        if (client_fd >= 0) {
            if (fork() == 0) {
                close(server_fd);
                serve_client(client_fd);
                exit(0);
            }
            close(client_fd);
        }
    }

    return 0;
}
