#!/usr/bin/python
# -*- coding: utf-8 -*-   
# 用Mininet做的仿真实验，结合了rstun项目（https://github.com/neevek/rstun）来调用quinn这个库                                                                 
import os
from mininet.topo import Topo
from mininet.net import Mininet
from mininet.node import CPULimitedHost, RemoteController
from mininet.link import TCLink
from mininet.util import dumpNodeConnections, ensureRoot
from mininet.log import setLogLevel
from mininet.clean import cleanup
from time import sleep

# QuicTunnle 通信
class Bbr2Topo(Topo):
    def build(self):
        # 创建交换机
        s1 = self.addSwitch('s1')
        s2 = self.addSwitch('s2')
        # s3 = self.addSwitch('s3')

        # 创建主机
        h1 = self.addHost('h1', ip = '10.0.0.1' )
        h2 = self.addHost('h2', ip = '10.0.0.2')
        h3 = self.addHost('h3', ip = '10.0.0.3')
        h4 = self.addHost('h4', ip = '10.0.0.4')
        h5 = self.addHost('h5', ip = '10.0.0.5')
        h6 = self.addHost('h6', ip = '10.0.0.6')
        h7 = self.addHost('h7', ip = '10.0.0.7')
        h8 = self.addHost('h8', ip = '10.0.0.8')

        # 添加链路
        # 设 520 是瓶颈链路的 2*BDP (600)
        self.addLink(h1, s1, bw=100, delay='1ms', loss=0, max_queue_size=600, use_htb=True)
        self.addLink(h2, s1, bw=100, delay='1ms', loss=0, max_queue_size=600, use_htb=True)
        self.addLink(h3, s2, bw=100, delay='1ms', loss=0, max_queue_size=600, use_htb=True)
        self.addLink(h4, s2, bw=100, delay='1ms', loss=0, max_queue_size=600, use_htb=True)
        self.addLink(s1, s2, bw=50, delay='50ms', loss=0, max_queue_size=50, use_htb=True)
        self.addLink(h5, s1, bw=100, delay='1ms', loss=0, max_queue_size=600, use_htb=True)
        self.addLink(h6, s1, bw=100, delay='1ms', loss=0, max_queue_size=600, use_htb=True)
        self.addLink(h7, s2, bw=100, delay='1ms', loss=0, max_queue_size=600, use_htb=True)
        self.addLink(h8, s2, bw=100, delay='1ms', loss=0, max_queue_size=600, use_htb=True)

# quic tunnel clients
def QuicClientCmd(node, seq):
    homepath = os.getcwd()
    output_dir = f'{homepath}/MininetOutput/'
    if not os.path.exists(output_dir):
        os.makedirs(output_dir)
    outfile = output_dir + '%s.out' % node.name
    errfile = output_dir + '%s.err' % node.name
    node.cmd( 'echo >', outfile)
    node.cmd( 'echo >', errfile)
    cmd = './rstun/target/debug/rstunc --mode OUT --server-addr 10.0.0.%s:6060  --password 123456  --udp-mapping 10.0.0.%s:9090^10.0.0.%s:10123  --loglevel D --udp-timeout-ms 60000' %(20 + seq, 30+seq, 10+seq)
    node.cmdPrint(cmd, 
                   '>', outfile,
                   '2>', errfile,
                    '&'  )
# quic tunnel servers
def QuicServerCmd(node, seq):
    cmd = './rstun/target/debug/rstund --addr 10.0.0.%s:6060  --password 123456  --udp-upstream 10.0.0.%s:10123  --loglevel D --udp-timeout-ms 60000' %(20+seq, 10 + seq)

def start_capture(switch, interface='s1-eth1', filter=''):
    """在交换机端口启动抓包"""
    cmd = f'sudo tcpdump -U -i {interface} -w ./{interface}.pcap {filter} &'
    print(f"抓包命令: {cmd}")
    switch.cmd(cmd)


def run():
    ensureRoot()
    cleanup()
    topo = Bbr2Topo()
    # controller = RemoteController('c0', ip='127.0.0.1', port=6653)
    net = Mininet(topo=topo, host=CPULimitedHost, link=TCLink)
    net.start()
    links = net.links
    for num in range(len(links)):
        print(links[num])
    print("Dumping host connections")
    dumpNodeConnections(net.hosts)
    h1, h2, h3, h4, s1, s2, h5, h6, h7, h8 = net.get( 'h1', 'h2', 'h3', 'h4', 's1', 's2', 'h5', 'h6', 'h7', 'h8')
    net.pingAll()
     # 抓包
    for intf in s1.intfList():
        if intf.name != 'lo':
            start_capture(s1, interface=intf.name, filter='')
    for intf in s2.intfList():
        if intf.name != 'lo':
            start_capture(s2, interface=intf.name, filter='')
    # iperf server 监听10123
    h4.cmd('iperf -u -s -p 10123 -f m -i 1 &> h4_log.txt &')
    # h4.cmd('iperf -s -p 10123 -f m -i 0.5 -w 100M -P 1 &> h4_log.txt &')
    sleep(1)
    # quic server 从6060收到数据转发给10123
    h3.cmd('./rstun/target/debug/rstund --addr 10.0.0.3:6060  --password 123456  --udp-upstream 10.0.0.4:10123  --loglevel D --udp-timeout-ms 120000 &> h3_log.txt &')
    # h3.cmd('./rstun/target/debug/rstund --addr 10.0.0.3:6060  --password 123456  --tcp-upstream 10.0.0.4:10123  --loglevel D --tcp-timeout-ms 30000 &> h3_log.txt &')
    sleep(1)
    # quic client 从9090收到数据，传输给quicserver，并告诉它要转发给10.0.0.4:10123
    h2.cmd('./rstun/target/debug/rstunc --mode OUT --server-addr 10.0.0.3:6060  --password 123456  --udp-mapping 10.0.0.2:9090^10.0.0.4:10123  --loglevel D --udp-timeout-ms 120000 &> h2_log.txt &')
    # h2.cmd('./rstun/target/debug/rstunc --mode OUT --server-addr 10.0.0.3:6060  --password 123456  --tcp-mapping 10.0.0.2:9090^10.0.0.4:10123  --loglevel D --tcp-timeout-ms 30000 &> h2_log.txt &')
    sleep(1)
    # # iperf client 将数据给9090
    h1.cmd('iperf -c 10.0.0.2 -p 9090 -u -b 30m -l 1200 -f m -i 1 -t 300 &> h1_log.txt &')
    # h1.cmd('iperf -c 10.0.0.2 -p 9090 -b 10m -f m -i 0.5 -t 300 -w 100M &> h1_log.txt &')
    sleep(3)


    # # iperf server 监听10123
    h8.cmd('iperf -u -s -p 10000 -f m -i 1 &> h8_log.txt &')
    # h4.cmd('iperf -s -p 10123 -f m -i 0.5 -w 100M -P 1 &> h4_log.txt &')
    sleep(1)
    # quic server 从6060收到数据转发给10123
    h7.cmd('./rstun-cubic/target/debug/rstund --addr 10.0.0.7:6060  --password 123456  --udp-upstream 10.0.0.8:10000  --loglevel D --udp-timeout-ms 120000 &> h7_log.txt &')
    # # h3.cmd('./rstun/target/debug/rstund --addr 10.0.0.3:6060  --password 123456  --tcp-upstream 10.0.0.4:10123  --loglevel D --tcp-timeout-ms 30000 &> h3_log.txt &')
    # sleep(1)
    # # quic client 从9090收到数据，传输给quicserver，并告诉它要转发给10.0.0.4:10123
    h6.cmd('./rstun-cubic/target/debug/rstunc --mode OUT --server-addr 10.0.0.7:6060  --password 123456  --udp-mapping 10.0.0.6:9090^10.0.0.8:10000  --loglevel D --udp-timeout-ms 120000 &> h6_log.txt &')
    # # h2.cmd('./rstun/target/debug/rstunc --mode OUT --server-addr 10.0.0.3:6060  --password 123456  --tcp-mapping 10.0.0.2:9090^10.0.0.4:10123  --loglevel D --tcp-timeout-ms 30000 &> h2_log.txt &')
    # sleep(1)
    # # iperf client 将数据给9090
    h5.cmd('iperf -c 10.0.0.6 -p 9090 -u -b 30m -l 1200 -f m -i 1 -t 300 &> h5_log.txt &')
    # h1.cmd('iperf -c 10.0.0.2 -p 9090 -b 10m -f m -i 0.5 -t 300 -w 100M &> h1_log.txt &')
    
    

    # h8.cmd('iperf3 -s -p 10000 -f m -i 1 &> h8_log.txt &')
    # sleep(1)
    # h5.cmd('iperf3 -c 10.0.0.8 -p 10000 -f m -i 1 -t 300 -C cubic &> h5_log.txt &')
    
    # h4.cmd('iperf3 -s -p 10000 -f m -i 1 --verbose &> h4_log.txt &')
    # sleep(1)
    # h1.cmd('iperf3 -c 10.0.0.4  -u -b 5m -p 10000 -f m -i 1  -t 300 --verbose &> h1_log.txt &')
    # h4.cmd('iperf -s -u -p 10000 -f m -i 1 &> h4_log.txt &')
    # sleep(1)
    # h1.cmd('iperf -u -c 10.0.0.4  -p 10000 -f m -b 5m -i 1 -t 300 &> h1_log.txt &')
    sleep(60)
    # for x in range(1):
    #     links[8].intf1.config(bw=10, delay='50ms', loss=0, max_queue_size=600, use_htb=True)
    #     # links[8].intf2.config(bw=0.5, delay='50ms', loss=2, max_queue_size=10000, use_htb=True)
    #     sleep(1)
    #     links[8].intf1.config(bw=20, delay='50ms', loss=0, max_queue_size=600, use_htb=True)
    #     # links[8].intf2.config(bw=5, delay='50ms', loss=2, max_queue_size=10000, use_htb=True)
    #     sleep(3)
    #     links[8].intf1.config(bw=30, delay='50ms', loss=2, max_queue_size=600, use_htb=True)
    #     # links[8].intf2.config(bw=10, delay='50ms', loss=2, max_queue_size=10000, use_htb=True)
    #     sleep(3)
    #     links[8].intf1.config(bw=100, delay='50ms', loss=2, max_queue_size=600, use_htb=True)
    #     # links[8].intf2.config(bw=20, delay='50ms', loss=2, max_queue_size=10000, use_htb=True)
    #     sleep(3)
    

    h1.cmd('kill -SIGINT $(pgrep -f iperf)')
    h2.cmd('kill -SIGINT $(pgrep -f rstunc)')
    h3.cmd('kill -SIGINT $(pgrep -f rstund)')
    h4.cmd('kill -SIGINT $(pgrep -f iperf)')
    h5.cmd('kill -SIGINT $(pgrep -f iperf)')
    h6.cmd('kill -SIGINT $(pgrep -f rstunc)')
    h7.cmd('kill -SIGINT $(pgrep -f rstund)')
    h8.cmd('kill -SIGINT $(pgrep -f iperf)')

    net.stop()


if __name__ == '__main__':
    setLogLevel('info')
    run()