#include <libproc.h>
#include <unistd.h>
#include <sys/resource.h>
#include <mach/mach_time.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <inttypes.h>
#include <limits.h>
static uint64_t ns(uint64_t ticks,mach_timebase_info_data_t tb) {
    __uint128_t scaled=(__uint128_t)ticks*tb.numer/tb.denom;
    return scaled>UINT64_MAX ? UINT64_MAX:(uint64_t)scaled;
}
static void *burn(void *unused) {
    (void)unused;mach_timebase_info_data_t tb;mach_timebase_info(&tb);
    uint64_t start=mach_absolute_time();volatile uint64_t value=1;
    while(ns(mach_absolute_time()-start,tb)<250000000) value=value*1664525+1013904223;
    return NULL;
}
int main(int argc,char **argv) {
    if(argc!=2) return 2;
    mach_timebase_info_data_t tb;if(mach_timebase_info(&tb)!=KERN_SUCCESS || tb.denom==0) return 1;
    if(!strcmp(argv[1],"--self-check")) {
        struct rusage before,after;struct rusage_info_v4 a={0},b={0};
        getrusage(RUSAGE_SELF,&before);if(proc_pid_rusage(getpid(),RUSAGE_INFO_V4,(rusage_info_t*)&a)) return 1;
        uint64_t start=mach_absolute_time();pthread_t threads[4];
        for(int i=0;i<4;i++) if(pthread_create(&threads[i],NULL,burn,NULL)) return 1;
        for(int i=0;i<4;i++) pthread_join(threads[i],NULL);
        if(proc_pid_rusage(getpid(),RUSAGE_INFO_V4,(rusage_info_t*)&b)) return 1;getrusage(RUSAGE_SELF,&after);
        double observed=ns((b.ri_user_time+b.ri_system_time)-(a.ri_user_time+a.ri_system_time),tb)/1e9;
        double reference=(after.ru_utime.tv_sec-before.ru_utime.tv_sec)+(after.ru_stime.tv_sec-before.ru_stime.tv_sec)+((after.ru_utime.tv_usec-before.ru_utime.tv_usec)+(after.ru_stime.tv_usec-before.ru_stime.tv_usec))/1e6;
        double ratio=observed/reference;int ok=ratio>0.95 && ratio<1.05;
        printf("{\"ok\":%s,\"procCPUSeconds\":%.6f,\"getrusageCPUSeconds\":%.6f,\"wallSeconds\":%.6f,\"ratio\":%.6f,\"threads\":4,\"timebaseNumer\":%u,\"timebaseDenom\":%u}\n",ok ? "true":"false",observed,reference,ns(mach_absolute_time()-start,tb)/1e9,ratio,tb.numer,tb.denom);return ok ? 0:1;
    }
    struct rusage_info_v4 r={0};
    if(proc_pid_rusage(atoi(argv[1]),RUSAGE_INFO_V4,(rusage_info_t*)&r)!=0) return 1;
    uint64_t raw=r.ri_user_time+r.ri_system_time;
    printf("{\"cpuNS\":%" PRIu64 ",\"cpuRawMach\":%" PRIu64 ",\"timebaseNumer\":%u,\"timebaseDenom\":%u,\"footprintBytes\":%" PRIu64 ",\"residentBytes\":%" PRIu64 ",\"peakFootprintBytes\":%" PRIu64 ",\"packageIdleWakeups\":%" PRIu64 ",\"interruptWakeups\":%" PRIu64 ",\"diskReadBytes\":%" PRIu64 ",\"diskWriteBytes\":%" PRIu64 ",\"logicalWriteBytes\":%" PRIu64 "}\n",ns(raw,tb),raw,tb.numer,tb.denom,r.ri_phys_footprint,r.ri_resident_size,r.ri_lifetime_max_phys_footprint,r.ri_pkg_idle_wkups,r.ri_interrupt_wkups,r.ri_diskio_bytesread,r.ri_diskio_byteswritten,r.ri_logical_writes);
    return 0;
}
