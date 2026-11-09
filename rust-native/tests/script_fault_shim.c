/* Only injected into a disposable script-probe. Mandatory trace proves the
 * real OS paths were intercepted; no production/runtime fault hooks. */
#include <sys/stat.h>
#include <sys/wait.h>
#include <fcntl.h>
#include <unistd.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <stdio.h>
#include <stdarg.h>
#include <signal.h>
#include <libproc.h>

static int active(void) { return strcmp(getprogname(),"script-probe")==0; }
static void trace(const char *kind, long a, long b) {
    const char *path=getenv("RUYCAST_SCRIPT_TRACE");
    if(!path || !active())return;
    int saved=errno;
    char line[128];int n=snprintf(line,sizeof(line),"%s %ld %ld\n",kind,a,b);
    int fd=open(path,O_WRONLY|O_CREAT|O_APPEND,0600);
    if(fd>=0){write(fd,line,(size_t)n);close(fd);}
    errno=saved;
}
static int fault(const char *name) {
    const char *mode=getenv("RUYCAST_SCRIPT_FAULT");
    return active() && mode && strcmp(mode,name)==0;
}
static int shim_fcntl(int fd,int command,...) {
    struct stat st;
    if(active() && fd>2 && (command==F_GETFL || command==F_SETFL) &&
       fstat(fd,&st)==0 && S_ISFIFO(st.st_mode) &&
       ((command==F_GETFL && fault("getfl")) || (command==F_SETFL && fault("setfl")))) {
        trace("FAULT",command,fd);errno=EIO;return -1;
    }
    switch(command) {
        case F_GETFD:case F_GETFL:case F_GETOWN:return fcntl(fd,command);
        case F_DUPFD:case F_DUPFD_CLOEXEC:case F_SETFD:case F_SETFL:
        case F_SETOWN:case F_NOCACHE:case F_RDAHEAD:case F_SETNOSIGPIPE: {
            va_list args;va_start(args,command);int value=va_arg(args,int);va_end(args);
            return fcntl(fd,command,value);
        }
        case F_GETPATH: {
            va_list args;va_start(args,command);void *value=va_arg(args,void *);va_end(args);
            return fcntl(fd,command,value);
        }
        default:errno=EINVAL;return -1;
    }
}
static int shim_kill(pid_t pid,int sig) {
    trace("KILL",pid,sig);
    if(pid<0 && fault("kill-eperm")){trace("FAULTKILL",pid,sig);errno=EPERM;return -1;}
    return kill(pid,sig);
}
static int shim_proc_listpids(uint32_t type,uint32_t info,void *buffer,int size) {
    if(fault("list-error")){trace("FAULTLIST",type,info);errno=EIO;return 0;}
    if(fault("list-truncated")){trace("FAULTLIST",type,info);return size;}
    return proc_listpids(type,info,buffer,size);
}
static int shim_proc_pidinfo(int pid,int flavor,uint64_t arg,void *buffer,int size) {
    if(fault("info-error")){trace("FAULTINFO",pid,flavor);errno=EPERM;return 0;}
    return proc_pidinfo(pid,flavor,arg,buffer,size);
}
static pid_t shim_waitpid(pid_t pid,int *status,int options) {
    pid_t result=waitpid(pid,status,options);trace("WAIT",pid,result);return result;
}
static int shim_waitid(idtype_t type,id_t id,siginfo_t *info,int options) {
    static int injected=0;
    if(!injected && (fault("wait-error") || fault("wait-eintr"))) {
        injected=1;trace("FAULTWAIT",id,options);
        errno=fault("wait-error")?EIO:EINTR;return -1;
    }
    return waitid(type,id,info,options);
}
static ssize_t shim_read(int fd,void *buffer,size_t count) {
    struct stat st;
    static int injected=0;
    if(!injected && fd>2 && active() && fstat(fd,&st)==0 && S_ISFIFO(st.st_mode) &&
       (fcntl(fd,F_GETFL)&O_NONBLOCK)!=0 &&
       (fault("read-error") || fault("read-eintr"))) {
        injected=1;trace("FAULTREAD",fd,0);errno=fault("read-error")?EIO:EINTR;return -1;
    }
    return read(fd,buffer,count);
}
#define INTERPOSE(replacement,original) \
    __attribute__((used)) static const struct {const void *r;const void *o;} hook_##original \
    __attribute__((section("__DATA,__interpose")))={(const void *)replacement,(const void *)original}
INTERPOSE(shim_fcntl,fcntl);
INTERPOSE(shim_kill,kill);
INTERPOSE(shim_waitpid,waitpid);
INTERPOSE(shim_waitid,waitid);
INTERPOSE(shim_read,read);
INTERPOSE(shim_proc_listpids,proc_listpids);
INTERPOSE(shim_proc_pidinfo,proc_pidinfo);
