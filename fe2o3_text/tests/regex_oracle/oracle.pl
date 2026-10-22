#!/usr/bin/env perl
# Runs corpus.txt through Perl's regular expression engine and writes expected.txt, the answers
# tests/regex.rs holds fe2o3_text's engine to.
#
# Perl is the oracle for what a match is -- leftmost-first backtracking, captures, classes, Unicode
# properties.  Where Perl and the Rust `regex` crate differ in *policy* rather than in matching,
# this script carries the Rust policy itself, written independently of fe2o3_text:
#
#   - a search from a position sees the text before it as context (`^`, `\b`), done here by
#     consuming that text inside the pattern rather than by Perl's `pos`, whose rules for empty
#     matches are its own;
#   - iteration passes over an empty match where the previous match ended, and searches again one
#     character on;
#   - `split` yields the pieces between matches; `$name`, `${name}` and `$$` expand as the crate
#     documents.
#
# Run from this directory:  perl oracle.pl < corpus.txt > expected.txt
use strict;
use warnings;
use utf8;
use Encode qw(encode_utf8);

binmode STDIN,  ':encoding(UTF-8)';
binmode STDOUT, ':encoding(UTF-8)';

# Escapes a field for expected.txt: backslash, tab, newline and carriage return.
sub esc {
	my $s = shift;
	$s =~ s/\\/\\\\/g;
	$s =~ s/\t/\\t/g;
	$s =~ s/\n/\\n/g;
	$s =~ s/\r/\\r/g;
	return $s;
}

# Decodes a corpus haystack.
sub unhay {
	my $s = shift;
	$s =~ s/\\x\{([0-9A-Fa-f]+)\}/chr(hex($1))/ge;
	$s =~ s/\\n/\n/g;
	$s =~ s/\\t/\t/g;
	return $s;
}

# The leftmost match at or after character $p, as [[start, end] or undef per group] in characters,
# with the named groups' texts.
sub search {
	my ($re, $s, $p) = @_;
	return undef unless $s =~ /\A(?s:.{$p})(?s:.*?)\K$re/;
	my @g;
	for my $i (0 .. $#+) {
		push @g, defined $-[$i] ? [$-[$i], $+[$i]] : undef;
	}
	my %named;
	for my $k (keys %-) {
		$named{$k} = $-{$k}[0];
	}
	return { g => \@g, n => \%named };
}

# Every match, by the iteration rule of the Rust `regex` crate.
sub matches {
	my ($re, $s) = @_;
	my $len = length $s;
	my ($at, $last) = (0, undef);
	my @out;
	while ($at <= $len) {
		my $m = search($re, $s, $at);
		last unless $m;
		my ($b, $e) = @{ $m->{g}[0] };
		if ($b == $e && defined $last && $e == $last) {
			last if $at + 1 > $len;
			$m = search($re, $s, $at + 1);
			last unless $m;
		}
		push @out, $m;
		$at		= $m->{g}[0][1];
		$last	= $at;
	}
	return @out;
}

# Expands a replacement template for one match, as the `regex` crate's `Captures::expand` does.
sub expand {
	my ($tpl, $s, $m) = @_;
	my $out = '';
	my $text = sub {
		my $name = shift;
		if ($name =~ /^[0-9]+$/) {
			my $g = $m->{g}[$name];
			return defined $g ? substr($s, $g->[0], $g->[1] - $g->[0]) : '';
		}
		my $v = $m->{n}{$name};
		return defined $v ? $v : '';
	};
	while (length $tpl) {
		if ($tpl =~ s/^\$\$//) {
			$out .= '$';
		} elsif ($tpl =~ s/^\$\{([^}]+)\}//) {
			$out .= $text->($1);
		} elsif ($tpl =~ s/^\$([0-9A-Za-z_]+)//) {
			$out .= $text->($1);
		} else {
			$tpl =~ s/^(.)//s;
			$out .= $1;
		}
	}
	return $out;
}

my ($pat, $perl, $tpl);
my $cases = 0;
while (my $line = <STDIN>) {
	chomp $line;
	next if $line =~ /^#/ || $line !~ /\S/;
	my ($tag, $body) = $line =~ /^(\w) (.*)$/ or die "unreadable corpus line: $line\n";
	if ($tag eq 'P') {
		($pat, $perl, $tpl) = ($body, $body, undef);
	} elsif ($tag eq 'Q') {
		$perl = $body;
	} elsif ($tag eq 'R') {
		$tpl = $body;
	} elsif ($tag eq 'H') {
		my $s	= unhay($body);
		die "a '\$' pattern over a haystack ending in a newline: $pat\n"
			if $perl =~ /\$/ && $s =~ /\n\z/;
		my $re	= eval { qr/$perl/ } or die "Perl will not compile '$perl': $@";
		my $len	= length $s;
		# Byte offset of each character, and of the end.
		my @off = (0);
		for my $i (1 .. $len) {
			push @off, $off[-1] + length(encode_utf8(substr($s, $i - 1, 1)));
		}
		my $spans = sub {
			my $m = shift;
			return join ' ', map { defined $_ ? "$off[$_->[0]],$off[$_->[1]]" : '-' } @{ $m->{g} };
		};
		print "P\t", esc($pat), "\nH\t", esc($s), "\n";
		# The search from every character position.
		for my $p (0 .. $len) {
			my $m = search($re, $s, $p);
			print "A\t$off[$p]\t", ($m ? $spans->($m) : 'none'), "\n";
		}
		my @ms = matches($re, $s);
		print "M\t", $spans->($_), "\n" for @ms;
		# Split pieces, each escaped, separated by U+001F.
		my ($last, @pieces) = (0);
		for my $m (@ms) {
			push @pieces, substr($s, $last, $m->{g}[0][0] - $last);
			$last = $m->{g}[0][1];
		}
		push @pieces, substr($s, $last);
		print "S\t", join("\x1F", map { esc($_) } @pieces), "\n";
		if (defined $tpl) {
			my ($out, $l) = ('', 0);
			for my $m (@ms) {
				$out .= substr($s, $l, $m->{g}[0][0] - $l) . expand($tpl, $s, $m);
				$l = $m->{g}[0][1];
			}
			$out .= substr($s, $l);
			print "R\t", esc($tpl), "\t", esc($out), "\n";
		}
		print "E\n";
		$cases++;
	}
}
print STDERR "$cases cases\n";
