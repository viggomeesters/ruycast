/* Actual private-writer publication, restricted to our disposable profile. */
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <errno.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <limits.h>
static int test_renameat(int from,const char *source,int to,const char *destination) {
    const char *home=getenv("RUYCAST_ANALYTICS_FIXTURE_HOME");
    char path[PATH_MAX],profile[PATH_MAX],marker[PATH_MAX];
    if(home && home[0]=='/' && strcmp(destination,"analytics.json")==0 && fcntl(to,F_GETPATH,path)==0) {
        snprintf(profile,sizeof(profile),"%s/p",home);
        snprintf(marker,sizeof(marker),"%s/.ruycast-test-fixture",home);
        if(strcmp(path,profile)==0 && access(marker,F_OK)==0) {
            char hold[PATH_MAX],entered[PATH_MAX],release[PATH_MAX],fail[PATH_MAX];
            snprintf(hold,sizeof(hold),"%s/hold-analytics-write",home);
            snprintf(entered,sizeof(entered),"%s/analytics-write-entered",home);
            snprintf(release,sizeof(release),"%s/analytics-write-release",home);
            snprintf(fail,sizeof(fail),"%s/fail-analytics-write",home);
            if(access(hold,F_OK)==0) {
                int fd=open(entered,O_WRONLY|O_CREAT|O_TRUNC,0600);if(fd>=0){write(fd,"held",4);close(fd);}
                unsigned n=0;while(access(release,F_OK)!=0){if(++n>15000){errno=ETIMEDOUT;return -1;}usleep(1000);}
            }
            if(access(fail,F_OK)==0) {
                snprintf(marker,sizeof(marker),"%s/analytics-write-failed",home);
                int fd=open(marker,O_WRONLY|O_CREAT|O_TRUNC,0600);if(fd>=0){write(fd,"EIO",3);close(fd);}
                errno=EIO;return -1;
            }
        }
    }
    return renameat(from,source,to,destination);
}
_Static_assert(__builtin_types_compatible_p(__typeof__(&renameat),__typeof__(&test_renameat)),"renameat ABI");
__attribute__((used)) static const struct {const void *replacement;const void *original;} interpose
__attribute__((section("__DATA,__interpose")))={(const void *)test_renameat,(const void *)renameat};
