/* Disposable test process only. Rust File::sync_all uses fcntl on macOS. */
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <stdio.h>
#include <stdarg.h>
static int test_fcntl(int fd, int command, ...) {
    if (command == F_FULLFSYNC || command == F_BARRIERFSYNC) {
        const char *directory = getenv("RUYCAST_TEST_FSYNC_DIRECTORY");
        char path[4096]; struct stat info;
        if (directory && fstat(fd,&info)==0 && S_ISDIR(info.st_mode) &&
            fcntl(fd,F_GETPATH,path)==0 && strcmp(path,directory)==0) {
            const char *marker = getenv("RUYCAST_TEST_FSYNC_MARKER");
            int first = marker && access(marker,F_OK)!=0;
            if (marker) {int m=open(marker,O_WRONLY|O_CREAT|O_TRUNC,0600);if(m>=0){write(m,"injected",8);close(m);}}
            const char *release = getenv("RUYCAST_TEST_FSYNC_RELEASE");
            if(first && release){unsigned attempts=0;while(access(release,F_OK)!=0){if(++attempts>5000){errno=ETIMEDOUT;return -1;}usleep(1000);}}
            const char *delay = getenv("RUYCAST_TEST_FSYNC_DELAY_MS");
            if(first && delay){long ms=strtol(delay,NULL,10);if(ms>0 && ms<=1000)usleep((useconds_t)ms*1000);}
            if(getenv("RUYCAST_TEST_FSYNC_DELAY_ONLY"))return fcntl(fd,command);
            const char *mutation = getenv("RUYCAST_TEST_FSYNC_MUTATE");
            if (first && mutation) {
                char target[4096], backup[4096];
                if(strcmp(mutation,"selector")==0) {
                    snprintf(target,sizeof(target),"%s/config-location.json",directory);
                    const char *json=getenv("RUYCAST_TEST_SELECTOR_JSON");
                    int changed=open(target,O_WRONLY|O_CREAT|O_TRUNC,0600);
                    if(changed>=0){if(json)write(changed,json,strlen(json));close(changed);}
                } else if(strcmp(mutation,"target")==0) {
                    const char *name=getenv("RUYCAST_TEST_MUTATION_TARGET");
                    if(name){int changed=open(name,O_WRONLY|O_TRUNC);if(changed>=0){write(changed,"external fixture edit",21);close(changed);}}
                } else {
                snprintf(target,sizeof(target),"%s/config.json",directory);
                if(strcmp(mutation,"replace")==0){snprintf(backup,sizeof(backup),"%s/published.copy",directory);rename(target,backup);}
                int changed=open(target,O_WRONLY|O_CREAT|O_TRUNC,0600);
                if(changed>=0){write(changed,"external fixture edit",21);close(changed);}
                }
            }
            errno=EIO; return -1;
        }
        return fcntl(fd,command);
    }
    switch(command) {
        case F_GETFD: case F_GETFL: case F_GETOWN: return fcntl(fd,command);
        case F_GETPATH: {
            va_list args;va_start(args,command);void *value=va_arg(args,void *);va_end(args);
            return fcntl(fd,command,value);
        }
        case F_DUPFD: case F_DUPFD_CLOEXEC: case F_SETFD: case F_SETFL:
        case F_SETOWN: case F_NOCACHE: case F_RDAHEAD: case F_SETNOSIGPIPE: {
            va_list args;va_start(args,command);int value=va_arg(args,int);va_end(args);
            return fcntl(fd,command,value);
        }
        default: errno=EINVAL;return -1;
    }
}
__attribute__((used)) static const struct {const void *replacement;const void *original;} interpose
__attribute__((section("__DATA,__interpose")))={(const void *)test_fcntl,(const void *)fcntl};
