/* Faults affect only the isolated parent probe, never its Rust CLI helper. */
#include <sys/wait.h>
#include <unistd.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <fcntl.h>
#include <errno.h>
#include <signal.h>
static pid_t test_waitpid(pid_t pid,int *status,int options){
    static int injected=0;
    const char *marker=getenv("RUYCAST_SUPERVISION_WAIT_MARKER");
    if(!injected && marker && strcmp(getprogname(),"supervision-probe")==0 && pid>0 && options==0){
        injected=1;
        int fd=open(marker,O_WRONLY|O_CREAT|O_TRUNC,0600);
        if(fd>=0){write(fd,"injected",8);close(fd);}
        errno=EIO;return -1;
    }
    return waitpid(pid,status,options);
}
__attribute__((used)) static const struct{const void*r;const void*o;} hook
__attribute__((section("__DATA,__interpose")))={(const void*)test_waitpid,(const void*)waitpid};
static int test_kill(pid_t pid,int sig){
    const char *path=getenv("RUYCAST_SUPERVISION_KILL_TRACE");
    if(path && strcmp(getprogname(),"supervision-probe")==0){
        char line[80];int n=snprintf(line,sizeof(line),"%d %d\n",pid,sig);
        int fd=open(path,O_WRONLY|O_CREAT|O_APPEND,0600);
        if(fd>=0){write(fd,line,(size_t)n);close(fd);}
    }
    return kill(pid,sig);
}
__attribute__((used)) static const struct{const void*r;const void*o;} kill_hook
__attribute__((section("__DATA,__interpose")))={(const void*)test_kill,(const void*)kill};
