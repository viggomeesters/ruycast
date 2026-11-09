/* Private selected export folder only; no process-global failures. */
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <errno.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <limits.h>
#include <stdarg.h>
static const char *own(void){const char *h=getenv("RUYCAST_DASHBOARD_FIXTURE_HOME");char marker[PATH_MAX];if(!h||h[0]!='/')return NULL;snprintf(marker,sizeof(marker),"%s/.ruycast-test-fixture",h);return access(marker,F_OK)==0?h:NULL;}
static int exists(const char *h,const char *name){char p[PATH_MAX];snprintf(p,sizeof(p),"%s/%s",h,name);return access(p,F_OK)==0;}
static void mark(const char *h,const char *name){char p[PATH_MAX];snprintf(p,sizeof(p),"%s/%s",h,name);int fd=open(p,O_WRONLY|O_CREAT|O_TRUNC,0600);if(fd>=0){write(fd,"1",1);close(fd);}}
static int wait_for(const char *h,const char *name){for(unsigned n=0;n<10000;++n){if(exists(h,name))return 0;usleep(1000);}errno=ETIMEDOUT;return -1;}
static int sync_fault(int fd){
    const char *h=own();char p[PATH_MAX],dir[PATH_MAX];struct stat s;
    if(h&&fcntl(fd,F_GETPATH,p)==0&&fstat(fd,&s)==0){
        snprintf(dir,sizeof(dir),"%s/export",h);size_t length=strlen(dir);
        if(strncmp(p,dir,length)==0&&p[length]=='/'&&S_ISREG(s.st_mode)){
            if(exists(h,"hold-export-file-sync")){mark(h,"export-file-sync-entered");if(wait_for(h,"export-file-sync-release"))return -1;}
            if(exists(h,"fail-export-file-sync")){mark(h,"export-file-sync-failed");errno=EIO;return -1;}
        }
        if(strcmp(p,dir)==0&&S_ISDIR(s.st_mode)&&exists(h,"fail-export-parent-sync")){mark(h,"export-parent-sync-failed");errno=EIO;return -1;}
    }
    return 0;
}
static int test_fsync(int fd){if(sync_fault(fd))return -1;return fsync(fd);}
static int test_fcntl(int fd,int command,...){
    if(command==F_FULLFSYNC || command==F_BARRIERFSYNC){if(sync_fault(fd))return -1;return fcntl(fd,command);}
    switch(command){
        case F_GETFD:case F_GETFL:case F_GETOWN:return fcntl(fd,command);
        case F_GETPATH:{va_list args;va_start(args,command);void *v=va_arg(args,void*);va_end(args);return fcntl(fd,command,v);}
        case F_DUPFD:case F_DUPFD_CLOEXEC:case F_SETFD:case F_SETFL:case F_SETOWN:case F_NOCACHE:case F_RDAHEAD:case F_SETNOSIGPIPE:{va_list args;va_start(args,command);int v=va_arg(args,int);va_end(args);return fcntl(fd,command,v);}
        default:errno=EINVAL;return -1;
    }
}
static int test_renameat(int from,const char *source,int to,const char *destination){
    const char *h=own();char p[PATH_MAX],dir[PATH_MAX];
    if(h&&strcmp(destination,"diagnostics.json")==0&&fcntl(to,F_GETPATH,p)==0){snprintf(dir,sizeof(dir),"%s/export",h);
        if(strcmp(p,dir)==0){
            if(exists(h,"hold-export-rename")){mark(h,"export-rename-entered");if(wait_for(h,"export-rename-release"))return -1;}
            if(exists(h,"fail-export-rename")){mark(h,"export-rename-failed");errno=EIO;return -1;}
        }
    }
    return renameat(from,source,to,destination);
}
#define INTERPOSE(replacement,original) __attribute__((used)) static const struct{const void *a;const void*b;} pair_##original __attribute__((section("__DATA,__interpose")))={(const void*)replacement,(const void*)original}
INTERPOSE(test_fsync,fsync);
INTERPOSE(test_renameat,renameat);

INTERPOSE(test_fcntl,fcntl);
