/* Fault injection only in an explicitly configured disposable test process. */
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <stdio.h>
static int private_test_fsync(int fd) {
    const char *directory=getenv("RUYCAST_TEST_FSYNC_DIRECTORY");
    char path[4096];struct stat info;
    if(directory && fstat(fd,&info)==0 && S_ISDIR(info.st_mode) && fcntl(fd,F_GETPATH,path)==0 && strcmp(path,directory)==0) {
        const char *marker=getenv("RUYCAST_TEST_FSYNC_MARKER");
        if(marker) {int m=open(marker,O_WRONLY|O_CREAT|O_TRUNC,0600);if(m>=0) {write(m,"injected",8);close(m);}}
        const char *mutation=getenv("RUYCAST_TEST_FSYNC_MUTATE");
        if(mutation) {
            char target[4096],backup[4096];
            snprintf(target,sizeof(target),"%s/config.json",directory);
            if(strcmp(mutation,"replace")==0) {snprintf(backup,sizeof(backup),"%s/published.copy",directory);rename(target,backup);}
            int changed=open(target,O_WRONLY|O_CREAT|O_TRUNC,0600);
            if(changed>=0) {write(changed,"external fixture edit",21);close(changed);}
        }
        errno=EIO;return -1;
    }
    return fsync(fd);
}
__attribute__((used)) static const struct {const void *replacement;const void *original;} private_interpose
__attribute__((section("__DATA,__interpose")))={(const void *)private_test_fsync,(const void *)fsync};
