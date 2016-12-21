#!/usr/bin/env python3

import json
import subprocess
import argparse


def run(args, **kwargs):
    return subprocess.check_output(args, universal_newlines=True, **kwargs)


def main():
    # Expects one argument: the network interface name
    parser = argparse.ArgumentParser(description='fetches various information')
    parser.add_argument('iface', metavar='IFACE',
                        help='network interface to monitor')
    args = parser.parse_args()

    print(json.dumps({
        'hostname': get_hostname(),
        'nproc': get_nproc(),
        'uptime': get_uptime(),
        'memory': get_memory_info(),
        'disks': get_disks(),
        'network': get_traffic(args.iface)
    }))


def get_hostname():
    return run('hostname').strip()


def get_nproc():
    try:
        return int(run('nproc'))
    except:
        return None


def get_uptime():
    try:
        uptime = run('uptime').split(':')[-1]
        tokens = [float(token.strip()) for token in uptime.split(',')]
        return tokens
    except:
        return None


def get_memory_info():
    try:
        memory = run(['head', '-n', '4', '/proc/meminfo'])
        lines = [line.split() for line in memory.split('\n')]
        total = int(lines[0][1])

        if 'available' in lines[2][1]:
            available = int(lines[2][1])
        else:
            free = int(lines[1][1])
            cached = int(lines[3][1])
            available = free + cached

        used = total - available

        return {'used': used, 'total': total}
    except:
        return None


def get_disks():
    try:
        disks = [line.split() for line in run(['df', '-P']).split('\n')[1:]]
        disks = [disk for disk in disks
                 if disk and not disk[0] in ["tmpfs", "udev", "cgmfs", "none"]]
        return [{'device': disk[0],
                 'size': int(disk[1]),
                 'used': int(disk[2]),
                 'available': int(disk[3]),
                 'mount': disk[5]} for disk in disks]
    except:
        return []


def get_traffic(iface):
    result = {}

    try:
        network_ip = run(['ip', 'addr', 'show', iface, 'scope', 'global']).split('\n')[2].split()[1]
        result['ip'] = network_ip.split('/')[0]
    except:
        pass

    try:
        tokens = run(['ifstat', '-i', iface, '5', '1']).split('\n')[2].split()
        result['rx'] = float(tokens[0])
        result['tx'] = float(tokens[1])
    except:
        pass

    return result


if __name__ == '__main__':
    main()
