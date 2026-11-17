// SPDX-License-Identifier: MPL-2.0

#include "../common/test.h"
#include <fcntl.h>
#include <signal.h>
#include <sys/socket.h>
#include <unistd.h>

static void interrupt_wait(int signo)
{
	(void)signo;
}

FN_TEST(dontwait_does_not_block_or_change_socket_status)
{
	const int types[] = { SOCK_STREAM, SOCK_SEQPACKET };
	struct sigaction action = { .sa_handler = interrupt_wait };
	CHECK(sigemptyset(&action.sa_mask));
	struct sigaction old_action;
	CHECK(sigaction(SIGALRM, &action, &old_action));

	for (size_t i = 0; i < sizeof(types) / sizeof(types[0]); ++i) {
		int sockets[2];
		CHECK(socketpair(AF_UNIX, types[i], 0, sockets));
		char data[4096] = { 0 };
		struct iovec iov = { .iov_base = data,
				     .iov_len = sizeof(data) };
		struct msghdr message = { .msg_iov = &iov, .msg_iovlen = 1 };

		// An ignored flag is interrupted by SIGALRM rather than hanging the suite.
		alarm(1);
		TEST_ERRNO(recvmsg(sockets[0], &message, MSG_DONTWAIT), EAGAIN);
		alarm(0);
		TEST_RES(fcntl(sockets[0], F_GETFL), (_ret & O_NONBLOCK) == 0);

		CHECK(send(sockets[1], "x", 1, 0));
		TEST_RES(recv(sockets[0], data, sizeof(data),
			      MSG_DONTWAIT | MSG_PEEK),
			 _ret == 1 && data[0] == 'x');
		TEST_RES(recvmsg(sockets[0], &message, MSG_DONTWAIT),
			 _ret == 1);
		alarm(1);
		TEST_ERRNO(recvmsg(sockets[0], &message, MSG_DONTWAIT), EAGAIN);
		alarm(0);

		int send_buffer = 4096;
		CHECK(setsockopt(sockets[0], SOL_SOCKET, SO_SNDBUF,
				 &send_buffer, sizeof(send_buffer)));
		ssize_t result;
		alarm(1);
		do {
			result = sendmsg(sockets[0], &message, MSG_DONTWAIT);
		} while (result > 0);
		int saved_errno = errno;
		TEST_RES(result, _ret == -1 && saved_errno == EAGAIN);
		alarm(0);
		TEST_RES(fcntl(sockets[0], F_GETFL), (_ret & O_NONBLOCK) == 0);
		CHECK(close(sockets[0]));
		CHECK(close(sockets[1]));
	}
	CHECK(sigaction(SIGALRM, &old_action, NULL));
}
END_TEST()
